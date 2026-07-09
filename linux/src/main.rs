//! HyperLink Linux host daemon.
//!
//! Exposes a QUIC server, advertises service via mDNS, handles TOFU pairing
//! PIN confirmations, and manages paired connection sockets.

mod connection;
mod discovery;

#[cfg(feature = "video")]
mod video_pipeline;
#[cfg(feature = "video")]
mod video_window;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use hyperlink_protocol::config::DeviceConfig;

#[derive(Parser)]
#[command(name = "hyperlink-linux")]
#[command(version)]
#[command(about = "HyperLink Linux Host Daemon")]
struct Cli {
    /// Address to bind the QUIC server to.
    #[arg(short, long, default_value = "0.0.0.0:9900")]
    bind: SocketAddr,

    /// Device display name advertised over mDNS.
    #[arg(short, long, default_value = "Linux-Host")]
    name: String,

    /// Start in pairing mode to pair a new client companion.
    #[arg(short, long)]
    pair: bool,

    /// Custom path to load/save host credentials and paired devices.
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    // Determine config path.
    let config_path = match cli.config {
        Some(p) => p,
        None => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config/hyperlink/host_config.json")
        }
    };

    info!("loading host configuration from: {:?}", config_path);
    let host_config = DeviceConfig::load_or_create(&config_path, &cli.name)?;

    // Start mDNS advertisement.
    let _discovery = match discovery::start_advertisement(&host_config.device_name, cli.bind.port())
    {
        Ok(handle) => {
            info!("mDNS advertisement started successfully");
            Some(handle)
        }
        Err(e) => {
            error!("failed to start mDNS advertisement: {}", e);
            None
        }
    };

    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║           HyperLink Host Daemon — Phase 2               ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();
    println!("  Device Name:   {}", host_config.device_name);
    println!("  Listening on:  {}", cli.bind);
    println!(
        "  Mode:          {}",
        if cli.pair {
            "Pairing Mode"
        } else {
            "Normal Mode"
        }
    );
    println!("  Press Ctrl+C to stop.");
    println!();

    // Start the server and wait for connections.
    #[cfg(feature = "video")]
    {
        run_with_gui(cli.bind, host_config, config_path, cli.pair)?;
    }
    #[cfg(not(feature = "video"))]
    {
        tokio::select! {
            res = connection::start_server(cli.bind, host_config, config_path, cli.pair) => {
                if let Err(e) = res {
                    error!("server failed: {}", e);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("shutting down server daemon");
            }
        }
    }

    Ok(())
}

#[cfg(feature = "video")]
pub enum VideoGuiMessage {
    Config {
        sps: Vec<u8>,
        pps: Vec<u8>,
    },
    Frame {
        data: Vec<u8>,
        timestamp_us: u64,
        is_keyframe: bool,
    },
}

#[cfg(feature = "video")]
pub static UI_SENDER: std::sync::OnceLock<gtk4::glib::Sender<VideoGuiMessage>> =
    std::sync::OnceLock::new();

#[cfg(feature = "video")]
fn run_with_gui(
    cli_bind: SocketAddr,
    host_config: DeviceConfig,
    config_path: PathBuf,
    is_pairing: bool,
) -> anyhow::Result<()> {
    use gtk4::prelude::*;
    use gtk4::Application;

    let app = Application::builder()
        .application_id("com.hyperlink.host")
        .build();

    app.connect_activate(move |app| {
        let (sender, receiver) =
            gtk4::glib::MainContext::channel::<VideoGuiMessage>(gtk4::glib::Priority::default());
        if UI_SENDER.set(sender).is_err() {
            error!("failed to initialize UI_SENDER");
        }

        let mut pipeline_opt: Option<video_pipeline::VideoPipeline> = None;
        let mut window_opt: Option<libadwaita::ApplicationWindow> = None;

        let app_clone = app.clone();
        receiver.attach(None, move |msg| {
            match msg {
                VideoGuiMessage::Config { sps, pps } => {
                    if pipeline_opt.is_none() {
                        match video_pipeline::VideoPipeline::new(true) {
                            Ok(pipeline) => match pipeline.paintable() {
                                Ok(paintable) => {
                                    let window =
                                        video_window::create_video_window(&app_clone, &paintable);
                                    window_opt = Some(window);
                                    pipeline_opt = Some(pipeline);
                                }
                                Err(e) => {
                                    error!("failed to get paintable from video pipeline: {}", e)
                                }
                            },
                            Err(e) => error!("failed to create video pipeline: {}", e),
                        }
                    }
                    if let Some(ref pipeline) = pipeline_opt {
                        pipeline.set_codec_data(&sps, &pps);
                        let _ = pipeline.start();
                    }
                }
                VideoGuiMessage::Frame {
                    data,
                    timestamp_us,
                    is_keyframe,
                } => {
                    if let Some(ref mut pipeline) = pipeline_opt {
                        pipeline.push_frame(&data, timestamp_us, is_keyframe);
                        if let Some(ref window) = window_opt {
                            let fps = 30.0;
                            let bitrate_kbps = (data.len() * 8 * 30 / 1000) as u32;
                            video_window::update_stats_label(window, fps, bitrate_kbps, 0.0);
                        }
                    }
                }
            }
            gtk4::glib::ControlFlow::Continue
        });
    });

    // Spawn QUIC server in background thread using dedicated Tokio runtime
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            if let Err(e) =
                connection::start_server(cli_bind, host_config, config_path, is_pairing).await
            {
                error!("server failed: {}", e);
            }
        });
    });

    app.run_with_args(&[""]);
    Ok(())
}
