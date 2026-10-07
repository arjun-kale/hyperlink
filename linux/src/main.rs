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
pub mod video_assembler;

#[cfg(feature = "video")]
mod app_window;
#[cfg(feature = "video")]
mod glass;
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
    /// Ask the phone for a keyframe after the host lost a video frame.
    KeyframeRequest,
}

#[cfg(feature = "video")]
pub static INPUT_SENDER: std::sync::OnceLock<async_channel::Sender<InputGuiMessage>> =
    std::sync::OnceLock::new();
#[cfg(feature = "video")]
pub static INPUT_RECEIVER: std::sync::OnceLock<async_channel::Receiver<InputGuiMessage>> =
    std::sync::OnceLock::new();

/// Session lifecycle the GUI reflects (sent from the connection handler).
#[derive(Debug, Clone)]
pub enum SessionEvent {
    PhoneConnected {
        device_name: String,
    },
    PhoneDisconnected,
    /// The phone runs an incompatible HyperLink version and was disconnected.
    ProtocolMismatch {
        peer_app_version: String,
    },
}

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

/// The name this computer goes by: its "pretty" hostname if set (e.g.
/// "Arjun's Laptop"), otherwise the plain hostname.
pub fn default_computer_name() -> String {
    let pretty = std::fs::read_to_string("/etc/machine-info")
        .ok()
        .and_then(|info| {
            info.lines()
                .find_map(|l| l.strip_prefix("PRETTY_HOSTNAME="))
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .filter(|v| !v.is_empty());
    pretty
        .or_else(|| {
            std::fs::read_to_string("/proc/sys/kernel/hostname")
                .ok()
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_else(|| "Linux computer".to_string())
}

/// The proximity listener's manager, so trust changes made while running
/// (pairing in the app, removing a phone) reach it without a restart.
static PROXIMITY: std::sync::OnceLock<std::sync::Arc<proximity::HostProximityManager>> =
    std::sync::OnceLock::new();

/// Updates which phones the proximity listener treats as trusted.
pub fn refresh_proximity_trust(peers: &std::collections::HashMap<String, String>) {
    if let Some(mgr) = PROXIMITY.get() {
        mgr.set_trusted_peers(peers.clone());
    }
}

/// The name older versions gave every computer; replaced by the real one.
const LEGACY_DEFAULT_NAME: &str = "Linux-Host";

/// Posts a plain desktop notification (no phone notification behind it).
pub fn dispatch_simple_desktop_notification(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .arg("-a")
        .arg("HyperLink")
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

    /// Name phones see for this computer. Defaults to the computer's own name.
    #[arg(short, long)]
    name: Option<String>,

    /// Start in pairing mode to pair a new client companion.
    #[arg(short, long)]
    pair: bool,

    /// Custom path to load/save host credentials and paired devices.
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Run a natural language query against the ambient context agent and exit.
    #[arg(long)]
    agent_query: Option<String>,

    /// Start without showing the window (for login autostart). Launching
    /// HyperLink again brings the window up.
    #[arg(long)]
    background: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // rustls 0.23 requires an explicit process-level CryptoProvider — without this,
    // the first TLS operation (the QUIC handshake) panics at runtime. cargo test
    // never caught this because protocol/src/crypto.rs's test installs one itself;
    // the real binary never did. Confirmed by actually running the daemon, not by
    // `cargo build`/`cargo test`, which don't exercise this path at all.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls ring CryptoProvider");

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

    // One running instance: launching HyperLink again just shows its window,
    // rather than failing on the already-bound port.
    #[cfg(feature = "video")]
    let gtk_app = {
        use gtk4::prelude::*;
        let app = libadwaita::Application::builder()
            .application_id("com.hyperlink.Host")
            .build();
        app.register(None::<&gtk4::gio::Cancellable>)?;
        if app.is_remote() {
            app.activate();
            info!("HyperLink is already running; brought its window forward");
            return Ok(());
        }
        app
    };

    // Determine config path.
    let config_path = match cli.config {
        Some(p) => p,
        None => PathBuf::from(home).join(".config/hyperlink/host_config.json"),
    };

    info!("loading host configuration from: {:?}", config_path);
    let mut host_config = DeviceConfig::load_or_create(
        &config_path,
        cli.name.as_deref().unwrap_or(&default_computer_name()),
    )?;
    // An explicit --name wins; otherwise replace the old generic default with
    // this computer's real name. Pairing is by certificate, so renaming is safe.
    let wanted_name = match cli.name {
        Some(name) => Some(name),
        None if host_config.device_name == LEGACY_DEFAULT_NAME => Some(default_computer_name()),
        None => None,
    };
    if let Some(name) = wanted_name.filter(|n| *n != host_config.device_name) {
        host_config.device_name = name;
        host_config.save(&config_path)?;
    }

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
    let _ = PROXIMITY.set(proximity_mgr.clone());
    let _oob_listener = proximity_mgr.start_out_of_band_beacon_listener(cli.bind.port() + 1);

    println!();
    println!(
        "  HyperLink is running as “{}” on {}",
        host_config.device_name, cli.bind
    );
    if cli.pair {
        println!("  Pairing is open: tap this computer in the HyperLink app on your phone.");
    }
    println!("  Press Ctrl+C to stop.");
    println!();

    // Start the server and wait for connections.
    #[cfg(feature = "video")]
    {
        run_with_gui(
            gtk_app,
            cli.bind,
            host_config,
            config_path,
            cli.pair,
            cli.background,
        )?;
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
    /// A frame was decoded-bound (it already went straight to the pipeline);
    /// this only carries what the UI needs for stats and sizing.
    Frame {
        bytes: usize,
        width: u16,
        height: u16,
    },
    Notification(hyperlink_protocol::notification::NotificationPost),
    NotificationDismiss(String),
    DndSync(bool),
    Session(SessionEvent),
    PairingRequest {
        pin: u32,
        reply: tokio::sync::oneshot::Sender<bool>,
    },
}

/// Where the network task pushes encoded frames, once a stream is set up.
#[cfg(feature = "video")]
static VIDEO_INPUT: std::sync::Mutex<Option<video_pipeline::VideoInput>> =
    std::sync::Mutex::new(None);

#[cfg(feature = "video")]
pub fn video_input() -> Option<video_pipeline::VideoInput> {
    VIDEO_INPUT.lock().unwrap().clone()
}

#[cfg(feature = "video")]
pub static UI_SENDER: std::sync::OnceLock<async_channel::Sender<VideoGuiMessage>> =
    std::sync::OnceLock::new();

/// Forwards desktop clipboard changes (text, or PNG images) to the clipboard
/// watchers, using GTK's change notifications instead of polling.
///
/// On GNOME Wayland, the compositor only reports clipboard changes to the
/// focused app, so copies made in other apps sync when HyperLink is focused.
#[cfg(feature = "video")]
fn watch_desktop_clipboard(tx: tokio::sync::broadcast::Sender<clipboard::LocalClipboardItem>) {
    use gtk4::prelude::*;
    let Some(display) = gtk4::gdk::Display::default() else {
        return;
    };
    display.clipboard().connect_changed(move |cb| {
        let cb = cb.clone();
        let tx = tx.clone();
        gtk4::glib::spawn_future_local(async move {
            let formats = cb.formats();
            let item = if formats.contain_mime_type("image/png") {
                match cb.read_texture_future().await {
                    Ok(Some(texture)) => Some(clipboard::LocalClipboardItem::Image {
                        mime_type: "image/png".to_string(),
                        bytes: texture.save_to_png_bytes().to_vec(),
                    }),
                    _ => None,
                }
            } else {
                match cb.read_text_future().await {
                    Ok(Some(text)) if !text.is_empty() => {
                        Some(clipboard::LocalClipboardItem::Text(text.to_string()))
                    }
                    _ => None,
                }
            };
            if let Some(item) = item {
                let _ = tx.send(item);
            }
        });
    });
}

#[cfg(feature = "video")]
fn run_with_gui(
    app: libadwaita::Application,
    cli_bind: SocketAddr,
    host_config: DeviceConfig,
    config_path: PathBuf,
    is_pairing: bool,
    start_hidden: bool,
) -> anyhow::Result<()> {
    use gtk4::prelude::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    let pc_name = host_config.device_name.clone();
    let config_path_for_gui = config_path.clone();

    // Clipboard changes come from GTK (no polling); set up before the server
    // starts so its clipboard watchers subscribe instead of polling.
    let (clipboard_tx, _) = tokio::sync::broadcast::channel(8);
    let _ = clipboard::LOCAL_CLIPBOARD.set(clipboard_tx.clone());
    let main_window: Rc<RefCell<Option<Rc<app_window::AppWindow>>>> = Rc::new(RefCell::new(None));

    app.connect_activate(move |app| {
        // Later activations (HyperLink launched again) just bring the window up.
        if let Some(window) = main_window.borrow().as_ref() {
            window.window.present();
            return;
        }

        // Keeps HyperLink running while its window is closed: the phone stays
        // linked in the background. Held for the life of the process.
        std::mem::forget(app.hold());

        watch_desktop_clipboard(clipboard_tx.clone());

        let window = app_window::AppWindow::new(app, config_path_for_gui.clone(), pc_name.clone());
        if !start_hidden {
            window.window.present();
        }
        #[cfg(debug_assertions)]
        if let Some(dir) = std::env::var_os("HYPERLINK_UI_SNAPSHOTS") {
            window.window.present();
            window.capture_ui_snapshots(
                dir.into(),
                std::env::var_os("HYPERLINK_UI_SAMPLE").map(Into::into),
            );
        }
        main_window.replace(Some(window.clone()));

        let (sender, receiver) = async_channel::unbounded::<VideoGuiMessage>();
        if UI_SENDER.set(sender).is_err() {
            error!("failed to initialize UI_SENDER");
        }

        gtk4::glib::spawn_future_local(async move {
            let mut pipeline_opt: Option<video_pipeline::VideoPipeline> = None;
            let mut frame_timestamps: std::collections::VecDeque<std::time::Instant> =
                std::collections::VecDeque::new();
            let mut byte_history: std::collections::VecDeque<(std::time::Instant, usize)> =
                std::collections::VecDeque::new();

            while let Ok(msg) = receiver.recv().await {
                match msg {
                    VideoGuiMessage::Config { sps, pps } => {
                        if pipeline_opt.is_none() {
                            match video_pipeline::VideoPipeline::new(true) {
                                Ok(pipeline) => pipeline_opt = Some(pipeline),
                                Err(e) => error!("failed to create video pipeline: {}", e),
                            }
                        }
                        if let Some(ref pipeline) = pipeline_opt {
                            match pipeline.paintable() {
                                Ok(paintable) => window.on_stream_started(&paintable),
                                Err(e) => {
                                    error!("failed to get paintable from video pipeline: {}", e)
                                }
                            }
                            // Codec data first, then open the direct path for frames.
                            pipeline.set_codec_data(&sps, &pps);
                            let _ = pipeline.start();
                            *VIDEO_INPUT.lock().unwrap() = Some(pipeline.input());
                            // Frames that raced ahead of the config were dropped; get a
                            // keyframe now rather than waiting for the next scheduled one.
                            if let Some(tx) = INPUT_SENDER.get() {
                                let _ = tx.try_send(InputGuiMessage::KeyframeRequest);
                            }
                        }
                    }
                    VideoGuiMessage::Frame {
                        bytes,
                        width,
                        height,
                    } => {
                        // 1-second rolling window for the optional stats readout.
                        let now = std::time::Instant::now();
                        frame_timestamps.push_back(now);
                        byte_history.push_back((now, bytes));
                        let one_sec_ago = now
                            .checked_sub(std::time::Duration::from_secs(1))
                            .unwrap_or(now);
                        while frame_timestamps.front().is_some_and(|&t| t < one_sec_ago) {
                            frame_timestamps.pop_front();
                        }
                        while byte_history.front().is_some_and(|&(t, _)| t < one_sec_ago) {
                            byte_history.pop_front();
                        }
                        let fps = frame_timestamps.len() as f64;
                        let total_bytes: usize = byte_history.iter().map(|(_, b)| *b).sum();
                        // Latency is deliberately not shown: it needs a calibrated
                        // cross-device clock offset that isn't wired in yet.
                        window.on_frame(width, height, fps, (total_bytes * 8 / 1000) as u32);
                    }
                    VideoGuiMessage::Notification(notif) => {
                        if is_dnd_active() {
                            info!(
                                id = %notif.id,
                                app = %notif.app_name,
                                "suppressing notification on Linux because DND is active"
                            );
                        } else if window.window.is_active() {
                            // Already looking at HyperLink: an in-window toast is enough.
                            video_window::show_notification_toast(
                                &window.window,
                                &window.toasts,
                                notif,
                            );
                        } else {
                            dispatch_desktop_notification(&notif);
                        }
                    }
                    VideoGuiMessage::NotificationDismiss(id) => {
                        debug!(id = %id, "notification dismissed on phone");
                    }
                    VideoGuiMessage::DndSync(enabled) => {
                        info!(dnd = enabled, "Do-Not-Disturb state synced from phone");
                        set_global_dnd_active(enabled);
                        if let Some(btn) = window.dnd_button() {
                            video_window::set_dnd_button_state(&btn, enabled);
                        }
                    }
                    VideoGuiMessage::Session(SessionEvent::PhoneConnected { device_name }) => {
                        window.on_phone_connected(&device_name);
                    }
                    VideoGuiMessage::Session(SessionEvent::PhoneDisconnected) => {
                        window.on_phone_disconnected();
                    }
                    VideoGuiMessage::Session(SessionEvent::ProtocolMismatch { peer_app_version }) => {
                        window.toast(&format!(
                            "Your phone has HyperLink {peer_app_version}, which doesn't work with this version ({}). Update both apps to the latest version.",
                            env!("CARGO_PKG_VERSION")
                        ));
                    }
                    VideoGuiMessage::PairingRequest { pin, reply } => {
                        window.on_pairing_request(pin, reply);
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
