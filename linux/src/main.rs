//! HyperLink Linux host daemon.
//!
//! Exposes a QUIC server, advertises service via mDNS, handles TOFU pairing
//! PIN confirmations, and manages paired connection sockets.

pub mod ambient;
pub mod clipboard;
mod connection;
pub mod crash_report;
mod discovery;
pub mod handoff;
pub mod multipath;
pub mod proximity;
pub mod update_check;
pub mod vfs;

#[cfg(feature = "video")]
mod preferences_window;
#[cfg(feature = "video")]
mod video_pipeline;
#[cfg(feature = "video")]
mod video_window;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
#[cfg(feature = "video")]
use tracing::debug;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use hyperlink_protocol::config::DeviceConfig;

#[cfg(feature = "video")]
#[derive(Debug, Clone)]
pub enum InputGuiMessage {
    Pointer(hyperlink_protocol::input::PointerEvent),
    Key(hyperlink_protocol::input::KeyEvent),
    Scroll(hyperlink_protocol::input::ScrollEvent),
    Nav(hyperlink_protocol::input::NavEvent),
    Dnd(hyperlink_protocol::notification::DndSync),
    NotificationAction(hyperlink_protocol::notification::NotificationActionInvoke),
    NotificationDismiss(hyperlink_protocol::notification::NotificationDismiss),
}

#[cfg(feature = "video")]
pub static INPUT_SENDER: std::sync::OnceLock<async_channel::Sender<InputGuiMessage>> =
    std::sync::OnceLock::new();
#[cfg(feature = "video")]
pub static INPUT_RECEIVER: std::sync::OnceLock<async_channel::Receiver<InputGuiMessage>> =
    std::sync::OnceLock::new();

static DND_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn is_dnd_active() -> bool {
    DND_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_global_dnd_active(enabled: bool) {
    DND_ACTIVE.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

pub fn dispatch_desktop_notification(notif: &hyperlink_protocol::notification::NotificationPost) {
    let app_name = if notif.app_name.is_empty() {
        "HyperLink"
    } else {
        &notif.app_name
    };
    let title = if notif.title.is_empty() {
        "Notification"
    } else {
        &notif.title
    };
    let body = &notif.body;

    let _ = std::process::Command::new("notify-send")
        .arg("-a")
        .arg(app_name)
        .arg(title)
        .arg(body)
        .spawn();
}

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

    /// Run a natural language query against the ambient context agent and exit.
    #[arg(long)]
    agent_query: Option<String>,
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

    // Determine journal path for ambient event bus
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let journal_path = PathBuf::from(&home).join(".local/share/hyperlink/ambient_events.jsonl");

    // Handle standalone ambient context agent query
    if let Some(query_str) = cli.agent_query {
        let bus = crate::ambient::AmbientEventBus::new(Some(journal_path.clone()));
        // Read historical events if file exists
        if let Ok(content) = std::fs::read_to_string(&journal_path) {
            for line in content.lines() {
                if let Ok(event) =
                    serde_json::from_str::<hyperlink_protocol::ambient::AmbientEvent>(line)
                {
                    let _ = bus.publish(event);
                }
            }
        }
        let agent = crate::ambient::AmbientContextAgent::new(
            bus,
            hyperlink_protocol::ambient::AgentConsentPolicy::default(),
        );
        let now_us = hyperlink_protocol::clock::now_us();
        let req = hyperlink_protocol::ambient::AgentQueryRequest {
            query_id: 1,
            agent_id: "cli_query".to_string(),
            query_text: query_str,
            time_window_start_us: now_us.saturating_sub(3_600_000_000), // Last 1 hour
            time_window_end_us: now_us,
        };
        let resp = agent.process_query(&req);
        println!("\n{}\n", resp.answer_text);
        return Ok(());
    }

    // Determine config path.
    let config_path = match cli.config {
        Some(p) => p,
        None => PathBuf::from(home).join(".config/hyperlink/host_config.json"),
    };

    info!("loading host configuration from: {:?}", config_path);
    let host_config = DeviceConfig::load_or_create(&config_path, &cli.name)?;

    // Phase 11: install the panic hook (local-only crash reports, never transmitted —
    // see docs/SECURITY_REVIEW.md) and kick off a best-effort, non-blocking update check.
    crash_report::install(
        host_config.preferences.crash_reporting_enabled,
        crash_report::default_reports_dir(),
    );
    if host_config.preferences.update_check_enabled {
        tokio::spawn(async {
            let result = update_check::check_for_update(env!("CARGO_PKG_VERSION")).await;
            if result.update_available {
                if let Some(latest) = &result.latest_version {
                    info!(
                        current = result.current_version,
                        latest = latest,
                        "a newer HyperLink release is available"
                    );
                }
            }
        });
    }

    // Initialize input event message channels.
    #[cfg(feature = "video")]
    {
        let (input_tx, input_rx) = async_channel::bounded::<InputGuiMessage>(256);
        let _ = INPUT_SENDER.set(input_tx);
        let _ = INPUT_RECEIVER.set(input_rx);
    }

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

    // Phase 8: Start out-of-band proximity beacon listener on UDP (bind port + 1)
    let workflow_path = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("workflow_state.json");
    let trusted_peers = host_config.trusted_peers.clone();
    let proximity_mgr = std::sync::Arc::new(crate::proximity::HostProximityManager::new(
        workflow_path,
        trusted_peers,
    ));
    let _oob_listener = proximity_mgr.start_out_of_band_beacon_listener(cli.bind.port() + 1);

    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║           HyperLink Host Daemon — Phase 3               ║");
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
        width: u16,
        height: u16,
    },
    Notification(hyperlink_protocol::notification::NotificationPost),
    NotificationDismiss(String),
    DndSync(bool),
}

#[cfg(feature = "video")]
pub static UI_SENDER: std::sync::OnceLock<async_channel::Sender<VideoGuiMessage>> =
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

    // The original `config_path` is still needed by `connection::start_server` below;
    // the GUI activation closure gets its own clone.
    let config_path_for_gui = config_path.clone();

    app.connect_activate(move |app| {
        let (sender, receiver) = async_channel::unbounded::<VideoGuiMessage>();
        if UI_SENDER.set(sender).is_err() {
            error!("failed to initialize UI_SENDER");
        }

        let app_clone = app.clone();
        let config_path_for_window = config_path_for_gui.clone();
        gtk4::glib::spawn_future_local(async move {
            let mut pipeline_opt: Option<video_pipeline::VideoPipeline> = None;
            let mut window_opt: Option<libadwaita::ApplicationWindow> = None;
            let mut toast_overlay_opt: Option<libadwaita::ToastOverlay> = None;
            let mut dnd_button_opt: Option<gtk4::ToggleButton> = None;
            let mut frame_timestamps: std::collections::VecDeque<std::time::Instant> =
                std::collections::VecDeque::new();
            let mut byte_history: std::collections::VecDeque<(std::time::Instant, usize)> =
                std::collections::VecDeque::new();

            while let Ok(msg) = receiver.recv().await {
                match msg {
                    VideoGuiMessage::Config { sps, pps } => {
                        if pipeline_opt.is_none() {
                            match video_pipeline::VideoPipeline::new(true) {
                                Ok(pipeline) => match pipeline.paintable() {
                                    Ok(paintable) => {
                                        let (window, toast_overlay, dnd_button) =
                                            video_window::create_video_window(
                                                &app_clone,
                                                &paintable,
                                                config_path_for_window.clone(),
                                            );
                                        window_opt = Some(window);
                                        toast_overlay_opt = Some(toast_overlay);
                                        dnd_button_opt = Some(dnd_button);
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
                        width,
                        height,
                    } => {
                        if width > 0 && height > 0 {
                            video_window::set_video_dimensions(width as u32, height as u32);
                        }
                        if let Some(ref mut pipeline) = pipeline_opt {
                            pipeline.push_frame(&data, timestamp_us, is_keyframe);
                            if let Some(ref window) = window_opt {
                                let now = std::time::Instant::now();
                                frame_timestamps.push_back(now);
                                byte_history.push_back((now, data.len()));

                                // Maintain 1-second rolling window for live stats
                                let one_sec_ago = now
                                    .checked_sub(std::time::Duration::from_secs(1))
                                    .unwrap_or(now);
                                while let Some(&t) = frame_timestamps.front() {
                                    if t < one_sec_ago {
                                        frame_timestamps.pop_front();
                                    } else {
                                        break;
                                    }
                                }
                                while let Some(&(t, _)) = byte_history.front() {
                                    if t < one_sec_ago {
                                        byte_history.pop_front();
                                    } else {
                                        break;
                                    }
                                }

                                let fps = frame_timestamps.len() as f64;
                                let total_bytes: usize = byte_history.iter().map(|(_, b)| *b).sum();
                                let bitrate_kbps = (total_bytes * 8 / 1000) as u32;

                                // Real cross-device glass-to-glass latency requires calibrated NTP clock-sync
                                // offset from Phase 0 (Android presentationTimeUs uses monotonic device clock).
                                // Omit latency stat from UI until clock sync offset is wired into session state.
                                let calibrated_latency_ms: Option<f64> = None;

                                video_window::update_stats_label(
                                    window,
                                    fps,
                                    bitrate_kbps,
                                    calibrated_latency_ms,
                                );
                            }
                        }
                    }
                    VideoGuiMessage::Notification(notif) => {
                        if is_dnd_active() {
                            info!(
                                id = %notif.id,
                                app = %notif.app_name,
                                "suppressing notification on Linux because DND is active"
                            );
                        } else {
                            if let (Some(ref window), Some(ref toast_overlay)) =
                                (&window_opt, &toast_overlay_opt)
                            {
                                video_window::show_notification_toast(
                                    window,
                                    toast_overlay,
                                    notif.clone(),
                                );
                            }
                            dispatch_desktop_notification(&notif);
                        }
                    }
                    VideoGuiMessage::NotificationDismiss(id) => {
                        debug!(id = %id, "notification dismissed on phone");
                    }
                    VideoGuiMessage::DndSync(enabled) => {
                        info!(dnd = enabled, "Do-Not-Disturb state synced from phone");
                        set_global_dnd_active(enabled);
                        if let Some(ref btn) = dnd_button_opt {
                            video_window::set_dnd_button_state(btn, enabled);
                        }
                    }
                }
            }
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
