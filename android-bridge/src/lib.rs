//! JNI Bridge for HyperLink Android Companion.
//!
//! Handles background QUIC client connections, TOFU certificate verification,
//! pairing PIN generation, mDNS discovery integration, and event polling.

#![allow(clippy::missing_safety_doc)]

pub mod ambient;
pub mod file_provider;
pub mod handoff;
pub mod multipath;
pub mod proximity;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jint, jlong, jstring};
use jni::JNIEnv;
use lazy_static::lazy_static;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use hyperlink_protocol::config::DeviceConfig;
use hyperlink_protocol::crypto::{self, PendingPairingState, TofuServerVerifier};

lazy_static! {
    /// Global Tokio runtime for running background tasks.
    static ref RUNTIME: Runtime = Runtime::new().unwrap();

    /// Global client state.
    static ref CLIENT_STATE: Arc<Mutex<ClientState>> = Arc::new(Mutex::new(ClientState::default()));
}

/// Events emitted by the Rust QUIC client to the Android app.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// pairing PIN generated.
    PairingPinGenerated(u32),
    /// Connection established.
    Connected,
    /// Connection closed or failed. Carries a short reason code (see
    /// `close_reason_code`) or, for failures before the connection was up, the
    /// error text.
    Disconnected(String),
    /// Both sides confirmed pairing; the host's identity is now trusted.
    Paired,
    /// The host lost a video frame and wants a keyframe to recover.
    KeyframeRequest,
    /// Heartbeat or message received.
    MessageReceived(u8, Vec<u8>),
    /// Video stream ready to accept frames.
    VideoStreamReady,
    /// Pointer event received from host.
    PointerEvent {
        action: u8,
        button: u8,
        x_norm: u16,
        y_norm: u16,
        pressure: u8,
    },
    /// Key event received from host.
    KeyEvent {
        action: u8,
        keycode: u32,
        modifiers: u8,
    },
    /// Scroll event received from host.
    ScrollEvent {
        dx: i16,
        dy: i16,
        x_norm: u16,
        y_norm: u16,
    },
    /// Navigation action received from host (Back, Home, Recents).
    NavEvent { action: u8 },
    /// Notification action button clicked on host (Phase 4).
    NotificationActionInvoke { key: String, action_id: u32 },
    /// Notification dismissed on host (Phase 4).
    NotificationDismiss { key: String },
    /// Do-Not-Disturb state synced from host (Phase 4).
    DndSync { enabled: bool },
    /// Clipboard content received from host (Phase 5).
    ClipboardReceived {
        origin_id: String,
        content_type: u8,
        mime_type: String,
        payload: Vec<u8>,
    },
    /// Path failover notification (Phase 7).
    PathFailover {
        from_path: u8,
        to_path: u8,
        reason: u8,
    },
    /// Path health update (Phase 7).
    PathHealthUpdated {
        path_id: u8,
        rtt_us: u32,
        active: bool,
    },
    /// Scoped app-state handoff received from host (Phase 9).
    HandoffReceived {
        session_id: u64,
        handoff_type: u8,
        app_id: String,
        uri: String,
        title: String,
        state_json: String,
    },
    /// Ambient-agent consent policy pushed from host (Phase 10).
    AgentConsentUpdated {
        allow_notifications: bool,
        allow_clipboard: bool,
        allow_raw_video: bool,
    },
}

/// Escapes a string for embedding in the hand-rolled JSON emitted by `pollEvent`.
/// The pre-existing arms below (clipboard mime_type, notification fields, etc.)
/// interpolate strings unescaped, which is fine only because those values come
/// from fixed, code-controlled sets. `title`/`uri`/`state_json` on a handoff are
/// much more free-form (arbitrary note titles, JSON-in-JSON), so this is used for
/// every new string field added here rather than repeating that shortcut.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn to_base64(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[(triple >> 18) & 0x3F] as char);
        out.push(CHARS[(triple >> 12) & 0x3F] as char);
        if chunk.len() > 1 {
            out.push(CHARS[(triple >> 6) & 0x3F] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[triple & 0x3F] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Global mutable state for the client.
#[derive(Default)]
struct ClientState {
    config: Option<DeviceConfig>,
    config_path: Option<PathBuf>,
    pending_pairing_fp: Option<[u8; 32]>,
    /// Pairing completes only once the user has confirmed on the phone *and*
    /// the host has accepted (signalled by it opening its first stream), so a
    /// host-side rejection never leaves the phone trusting a host that doesn't
    /// trust it back.
    pairing_user_confirmed: Option<String>,
    pairing_host_accepted: bool,
    event_rx: Option<mpsc::UnboundedReceiver<ClientEvent>>,
    event_tx: Option<mpsc::UnboundedSender<ClientEvent>>,
    control_tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
    connection: Option<quinn::Connection>,
    multipath: Arc<Mutex<multipath::AndroidMultipathManager>>,
    recent_clipboard_hashes: std::collections::VecDeque<[u8; 32]>,
    last_local_clip_write_us: u64,
    last_applied_remote_clip_us: u64,
    last_applied_remote_clip_seq: u64,
}

/// Applies inbound clipboard deduplication and freshness checks.
fn apply_inbound_clip(clip: &hyperlink_protocol::clipboard::ClipboardMessage) -> bool {
    let mut state = CLIENT_STATE.lock().unwrap();
    let is_echo = state.recent_clipboard_hashes.contains(&clip.content_hash);
    let is_stale = clip.timestamp_us > 0 && clip.timestamp_us < state.last_local_clip_write_us;
    let is_dup = clip.timestamp_us > 0 && clip.timestamp_us <= state.last_applied_remote_clip_us;

    if is_echo || is_stale || is_dup {
        false
    } else {
        if state.recent_clipboard_hashes.len() >= 64 {
            state.recent_clipboard_hashes.pop_front();
        }
        state.recent_clipboard_hashes.push_back(clip.content_hash);
        state.last_applied_remote_clip_us = clip.timestamp_us;
        state.last_applied_remote_clip_seq = clip.seq;
        true
    }
}

/// Helper to push events to the JNI queue.
fn emit_event(event: ClientEvent) {
    let state = CLIENT_STATE.lock().unwrap();
    if let Some(ref tx) = state.event_tx {
        let _ = tx.send(event);
    }
}

// --- JNI Bindings ---

/// Initialize the client config and logging.
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_initialize(
    mut env: JNIEnv,
    _class: JClass,
    storage_path: JString,
) {
    let storage_path: String = env.get_string(&storage_path).unwrap().into();

    // rustls 0.23 requires an explicit process-level CryptoProvider before any TLS
    // use — the same bug already found and fixed in linux/src/main.rs. Here it's
    // worse to diagnose: the panic happens inside a tokio::spawn'd task
    // (run_connection_task, via ClientConfig::builder()), so it doesn't crash the
    // app — it silently kills just that task, and the raw panic message never
    // reaches logcat (tracing-android only captures tracing:: calls, not the
    // default panic hook's stderr output). Confirmed on a real device: pairing
    // would hang forever on "Initiating connection..." with zero further log
    // output on either side, and zero bytes ever reaching the host's UDP socket
    // (checked via /proc/net/udp — rx_queue and drops both stayed 0).
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Initialize android logger.
    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::prelude::*;
        let _ = tracing_subscriber::registry()
            .with(tracing_android::layer("HyperLinkClient").unwrap())
            .try_init();
    }

    info!(
        "initializing client config at storage path: {}",
        storage_path
    );

    let path = Path::new(&storage_path).join("client_config.json");

    let mut state = CLIENT_STATE.lock().unwrap();
    if state.config.is_some() {
        return; // already initialized
    }

    match DeviceConfig::load_or_create(&path, "Android-Companion") {
        Ok(config) => {
            info!(
                "client config loaded successfully, device name: {}",
                config.device_name
            );
            state.config = Some(config);
            state.config_path = Some(path);
        }
        Err(e) => {
            error!("failed to load or create client config: {}", e);
        }
    }

    // Set up channel for client events.
    let (tx, rx) = mpsc::unbounded_channel();
    state.event_tx = Some(tx);
    state.event_rx = Some(rx);
}

/// Connect to a discovered host.
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_connectHost(
    mut env: JNIEnv,
    _class: JClass,
    host_ip: JString,
    port: jint,
    is_pairing: jboolean,
) {
    let host_ip: String = env.get_string(&host_ip).unwrap().into();
    let port = port as u16;
    let is_pairing = is_pairing != 0;

    info!(
        "connecting to host {}:{} (pairing={})",
        host_ip, port, is_pairing
    );

    let state = CLIENT_STATE.lock().unwrap();
    let config = match state.config {
        Some(ref c) => c.clone(),
        None => {
            error!("cannot connect: client config is not initialized");
            emit_event(ClientEvent::Disconnected("Config not initialized".into()));
            return;
        }
    };
    let event_tx = state.event_tx.clone();

    // Spawn async connection task.
    RUNTIME.spawn(async move {
        if let Err(e) = run_connection_task(host_ip, port, is_pairing, config, event_tx).await {
            error!("connection task failed: {}", e);
            emit_event(ClientEvent::Disconnected(e.to_string()));
        }
    });
}

/// Closes the current connection, if any (the user tapped Disconnect or
/// cancelled pairing). Reported back as a `closed_locally` disconnect.
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_disconnectHost(
    _env: JNIEnv,
    _class: JClass,
) {
    let state = CLIENT_STATE.lock().unwrap();
    if let Some(conn) = state.connection.as_ref() {
        conn.close(0u32.into(), b"user disconnected");
    }
}

/// Records the user's confirmation of the pairing code (and the name to trust
/// the host under). Trust is saved once the host has also accepted; see
/// `try_finish_pairing`. Returns false if there's no pairing in progress.
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_confirmPairing(
    mut env: JNIEnv,
    _class: JClass,
    host_name: JString,
) -> jboolean {
    let host_name: String = env.get_string(&host_name).unwrap().into();
    let mut state = CLIENT_STATE.lock().unwrap();
    if state.pending_pairing_fp.is_none() {
        warn!("cannot confirm pairing: no pairing in progress");
        return 0;
    }
    state.pairing_user_confirmed = Some(host_name);
    try_finish_pairing(&mut state);
    1
}

/// Persists the pending host fingerprint once both the user and the host have
/// accepted, then reports the session as connected.
fn try_finish_pairing(state: &mut ClientState) {
    if !state.pairing_host_accepted {
        return;
    }
    let Some(host_name) = state.pairing_user_confirmed.clone() else {
        return;
    };
    let (Some(fp), Some(mut config), Some(path)) = (
        state.pending_pairing_fp.take(),
        state.config.clone(),
        state.config_path.clone(),
    ) else {
        return;
    };
    state.pairing_user_confirmed = None;
    state.pairing_host_accepted = false;

    let fp_str = crypto::fingerprint_to_string(&fp);
    let key = config.add_trusted_peer_unique(&host_name, &fp_str);
    info!(
        "pairing complete: trusting host {:?} with fingerprint: {}",
        key, fp_str
    );
    if let Err(e) = config.save(&path) {
        error!("failed to save updated config: {}", e);
        return;
    }
    state.config = Some(config);

    // The session that paired is already live; tell the app directly. (Can't go
    // through emit_event here: it takes the state lock we're holding.)
    if let Some(ref tx) = state.event_tx {
        let _ = tx.send(ClientEvent::Paired);
        let _ = tx.send(ClientEvent::Connected);
        let _ = tx.send(ClientEvent::VideoStreamReady);
    }
}

/// The host opened a stream to us: during pairing, that means it accepted.
fn note_host_accepted() {
    let mut state = CLIENT_STATE.lock().unwrap();
    if state.pending_pairing_fp.is_some() && !state.pairing_host_accepted {
        state.pairing_host_accepted = true;
        try_finish_pairing(&mut state);
    }
}

/// Maps how the host closed the connection to a short code the app turns into
/// user-facing copy.
fn close_reason_code(err: &quinn::ConnectionError) -> String {
    match err {
        quinn::ConnectionError::ApplicationClosed(close) => match close.reason.as_ref() {
            b"pairing rejected" => "pairing_rejected",
            b"pairing timed out" => "pairing_timed_out",
            b"device revoked" => "device_revoked",
            b"pairing window closed" => "pairing_closed",
            _ => "host_closed",
        },
        quinn::ConnectionError::TimedOut => "timed_out",
        quinn::ConnectionError::LocallyClosed => "closed_locally",
        _ => "connection_lost",
    }
    .to_string()
}

/// Send a payload over the control stream.
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_sendMessage(
    env: JNIEnv,
    _class: JClass,
    payload: JByteArray,
) -> jboolean {
    let bytes = env.convert_byte_array(&payload).unwrap();
    let state = CLIENT_STATE.lock().unwrap();
    if let Some(ref tx) = state.control_tx {
        if tx.send(bytes).is_ok() {
            return 1; // true
        }
    }
    0 // false
}

/// Send a posted notification over the QUIC control stream (Phase 4).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_sendNotificationPost(
    mut env: JNIEnv,
    _class: JClass,
    id: JString,
    package_name: JString,
    app_name: JString,
    title: JString,
    body: JString,
    timestamp_ms: jlong,
    icon_bytes: JByteArray,
) -> jboolean {
    let id_str: String = match env.get_string(&id) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };
    let pkg_str: String = match env.get_string(&package_name) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };
    let app_str: String = match env.get_string(&app_name) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };
    let title_str: String = match env.get_string(&title) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };
    let body_str: String = match env.get_string(&body) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };
    let icon_opt = if !icon_bytes.is_null() {
        env.convert_byte_array(&icon_bytes).ok()
    } else {
        None
    };

    let notif = hyperlink_protocol::notification::NotificationPost {
        id: id_str,
        package_name: pkg_str,
        app_name: app_str,
        title: title_str,
        body: body_str,
        timestamp_ms: timestamp_ms as u64,
        actions: Vec::new(),
        icon_png: icon_opt,
    };

    let mut payload = Vec::new();
    if notif.encode(&mut payload).is_err() {
        return 0;
    }

    let header = hyperlink_protocol::version::Header::new(
        hyperlink_protocol::message::MessageType::NotificationPost,
        payload.len() as u32,
    );
    let mut packet = Vec::with_capacity(10 + payload.len());
    if header.encode(&mut packet).is_err() {
        return 0;
    }
    packet.extend_from_slice(&payload);

    let state = CLIENT_STATE.lock().unwrap();
    if let Some(ref tx) = state.control_tx {
        if tx.send(packet).is_ok() {
            return 1;
        }
    }
    0
}

/// Send notification dismiss event over the QUIC control stream (Phase 4).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_sendNotificationDismiss(
    mut env: JNIEnv,
    _class: JClass,
    id: JString,
) -> jboolean {
    let id_str: String = match env.get_string(&id) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };

    let dismiss = hyperlink_protocol::notification::NotificationDismiss { id: id_str };
    let mut payload = Vec::new();
    if dismiss.encode(&mut payload).is_err() {
        return 0;
    }

    let header = hyperlink_protocol::version::Header::new(
        hyperlink_protocol::message::MessageType::NotificationDismiss,
        payload.len() as u32,
    );
    let mut packet = Vec::with_capacity(10 + payload.len());
    if header.encode(&mut packet).is_err() {
        return 0;
    }
    packet.extend_from_slice(&payload);

    let state = CLIENT_STATE.lock().unwrap();
    if let Some(ref tx) = state.control_tx {
        if tx.send(packet).is_ok() {
            return 1;
        }
    }
    0
}

/// Send Do-Not-Disturb state synchronization over the QUIC control stream (Phase 4).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_sendDndSync(
    _env: JNIEnv,
    _class: JClass,
    enabled: jboolean,
) -> jboolean {
    let dnd = hyperlink_protocol::notification::DndSync {
        dnd_enabled: enabled != 0,
    };
    let mut payload = Vec::new();
    if dnd.encode(&mut payload).is_err() {
        return 0;
    }

    let header = hyperlink_protocol::version::Header::new(
        hyperlink_protocol::message::MessageType::DndSync,
        payload.len() as u32,
    );
    let mut packet = Vec::with_capacity(10 + payload.len());
    if header.encode(&mut packet).is_err() {
        return 0;
    }
    packet.extend_from_slice(&payload);

    let state = CLIENT_STATE.lock().unwrap();
    if let Some(ref tx) = state.control_tx {
        if tx.send(packet).is_ok() {
            return 1;
        }
    }
    0
}

/// Send text clipboard content to host (Phase 5).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_sendClipboardText(
    mut env: JNIEnv,
    _class: JClass,
    origin_id: JString,
    text: JString,
) -> jboolean {
    let origin_str: String = match env.get_string(&origin_id) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };
    let text_str: String = match env.get_string(&text) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };

    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);

    let msg = hyperlink_protocol::clipboard::ClipboardMessage::new_text(
        origin_str, &text_str, now_us, now_us,
    );

    let mut payload = Vec::new();
    if msg.encode(&mut payload).is_err() {
        return 0;
    }

    let header = hyperlink_protocol::version::Header::new(
        hyperlink_protocol::message::MessageType::ClipboardMessage,
        payload.len() as u32,
    );
    let mut packet = Vec::with_capacity(10 + payload.len());
    if header.encode(&mut packet).is_err() {
        return 0;
    }
    packet.extend_from_slice(&payload);

    let mut state = CLIENT_STATE.lock().unwrap();
    if state.recent_clipboard_hashes.len() >= 64 {
        state.recent_clipboard_hashes.pop_front();
    }
    state.recent_clipboard_hashes.push_back(msg.content_hash);
    state.last_local_clip_write_us = now_us;

    if let Some(ref tx) = state.control_tx {
        if tx.send(packet).is_ok() {
            return 1;
        }
    }
    0
}

/// Send image clipboard content to host (Phase 5).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_sendClipboardImage(
    mut env: JNIEnv,
    _class: JClass,
    origin_id: JString,
    mime_type: JString,
    image_bytes: JByteArray,
) -> jboolean {
    let origin_str: String = match env.get_string(&origin_id) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };
    let mime_str: String = match env.get_string(&mime_type) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };
    let bytes = match env.convert_byte_array(&image_bytes) {
        Ok(b) => b,
        Err(_) => return 0,
    };

    let now_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);

    let msg = hyperlink_protocol::clipboard::ClipboardMessage::new_image(
        origin_str, mime_str, bytes, now_us, now_us,
    );

    let mut payload = Vec::new();
    if msg.encode(&mut payload).is_err() {
        return 0;
    }

    let header = hyperlink_protocol::version::Header::new(
        hyperlink_protocol::message::MessageType::ClipboardMessage,
        payload.len() as u32,
    );
    let mut packet = Vec::with_capacity(10 + payload.len());
    if header.encode(&mut packet).is_err() {
        return 0;
    }
    packet.extend_from_slice(&payload);

    let mut state = CLIENT_STATE.lock().unwrap();
    if state.recent_clipboard_hashes.len() >= 64 {
        state.recent_clipboard_hashes.pop_front();
    }
    state.recent_clipboard_hashes.push_back(msg.content_hash);
    state.last_local_clip_write_us = now_us;

    if let Some(ref tx) = state.control_tx {
        if tx.send(packet).is_ok() {
            return 1;
        }
    }
    0
}

/// Poll events from the Rust client queue. Returns JSON string of event or null.
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_pollEvent(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let mut state = CLIENT_STATE.lock().unwrap();
    if let Some(ref mut rx) = state.event_rx {
        match rx.try_recv() {
            Ok(event) => {
                let json = match event {
                    ClientEvent::PairingPinGenerated(pin) => {
                        format!("{{\"type\":\"pairing_pin\",\"pin\":{}}}", pin)
                    }
                    ClientEvent::Connected => "{\"type\":\"connected\"}".to_string(),
                    ClientEvent::Disconnected(reason) => {
                        format!(
                            "{{\"type\":\"disconnected\",\"reason\":\"{}\"}}",
                            json_escape(&reason)
                        )
                    }
                    ClientEvent::Paired => "{\"type\":\"paired\"}".to_string(),
                    ClientEvent::KeyframeRequest => "{\"type\":\"keyframe_request\"}".to_string(),
                    ClientEvent::MessageReceived(msg_type, payload) => {
                        format!(
                            "{{\"type\":\"message\",\"message_type\":{},\"payload_len\":{}}}",
                            msg_type,
                            payload.len()
                        )
                    }
                    ClientEvent::VideoStreamReady => "{\"type\":\"video_ready\"}".to_string(),
                    ClientEvent::PointerEvent {
                        action,
                        button,
                        x_norm,
                        y_norm,
                        pressure,
                    } => {
                        format!(
                            "{{\"type\":\"pointer\",\"action\":{},\"button\":{},\"x_norm\":{},\"y_norm\":{},\"pressure\":{}}}",
                            action, button, x_norm, y_norm, pressure
                        )
                    }
                    ClientEvent::KeyEvent {
                        action,
                        keycode,
                        modifiers,
                    } => {
                        format!(
                            "{{\"type\":\"key\",\"action\":{},\"keycode\":{},\"modifiers\":{}}}",
                            action, keycode, modifiers
                        )
                    }
                    ClientEvent::ScrollEvent {
                        dx,
                        dy,
                        x_norm,
                        y_norm,
                    } => {
                        format!(
                            "{{\"type\":\"scroll\",\"dx\":{},\"dy\":{},\"x_norm\":{},\"y_norm\":{}}}",
                            dx, dy, x_norm, y_norm
                        )
                    }
                    ClientEvent::NavEvent { action } => {
                        format!("{{\"type\":\"nav\",\"action\":{}}}", action)
                    }
                    ClientEvent::NotificationActionInvoke { key, action_id } => {
                        format!(
                            "{{\"type\":\"notif_action\",\"key\":\"{}\",\"action_id\":{}}}",
                            key, action_id
                        )
                    }
                    ClientEvent::NotificationDismiss { key } => {
                        format!("{{\"type\":\"notif_dismiss\",\"key\":\"{}\"}}", key)
                    }
                    ClientEvent::DndSync { enabled } => {
                        format!("{{\"type\":\"dnd_sync\",\"enabled\":{}}}", enabled)
                    }
                    ClientEvent::ClipboardReceived {
                        origin_id,
                        content_type,
                        mime_type,
                        payload,
                    } => {
                        let b64 = to_base64(&payload);
                        format!(
                            "{{\"type\":\"clipboard\",\"origin_id\":\"{}\",\"content_type\":{},\"mime_type\":\"{}\",\"data_b64\":\"{}\"}}",
                            origin_id, content_type, mime_type, b64
                        )
                    }
                    ClientEvent::PathFailover {
                        from_path,
                        to_path,
                        reason,
                    } => {
                        format!(
                            "{{\"type\":\"path_failover\",\"from_path\":{},\"to_path\":{},\"reason\":{}}}",
                            from_path, to_path, reason
                        )
                    }
                    ClientEvent::PathHealthUpdated {
                        path_id,
                        rtt_us,
                        active,
                    } => {
                        format!(
                            "{{\"type\":\"path_health\",\"path_id\":{},\"rtt_us\":{},\"active\":{}}}",
                            path_id, rtt_us, active
                        )
                    }
                    ClientEvent::HandoffReceived {
                        session_id,
                        handoff_type,
                        app_id,
                        uri,
                        title,
                        state_json,
                    } => {
                        format!(
                            "{{\"type\":\"handoff\",\"session_id\":{},\"handoff_type\":{},\"app_id\":\"{}\",\"uri\":\"{}\",\"title\":\"{}\",\"state_json\":\"{}\"}}",
                            session_id,
                            handoff_type,
                            json_escape(&app_id),
                            json_escape(&uri),
                            json_escape(&title),
                            json_escape(&state_json)
                        )
                    }
                    ClientEvent::AgentConsentUpdated {
                        allow_notifications,
                        allow_clipboard,
                        allow_raw_video,
                    } => {
                        format!(
                            "{{\"type\":\"agent_consent\",\"allow_notifications\":{},\"allow_clipboard\":{},\"allow_raw_video\":{}}}",
                            allow_notifications, allow_clipboard, allow_raw_video
                        )
                    }
                };
                let jstr = env.new_string(json).unwrap();
                jstr.into_raw()
            }
            Err(_) => std::ptr::null_mut(),
        }
    } else {
        std::ptr::null_mut()
    }
}

// --- Async QUIC Client Logic ---

async fn run_connection_task(
    host_ip: String,
    port: u16,
    is_pairing: bool,
    config: DeviceConfig,
    _event_tx: Option<mpsc::UnboundedSender<ClientEvent>>,
) -> anyhow::Result<()> {
    // IPv6 literals need bracketing to be an unambiguous SocketAddr string — an
    // unbracketed "fe80::1:9900" is unparseable (which of those colons is the
    // port separator?). Confirmed on a real device: Android's mDNS resolution
    // can hand back an IPv6 link-local address, and the unbracketed format!()
    // that used to be here failed with exactly this "invalid socket address
    // syntax" on a real pairing attempt.
    let host_port = if host_ip.contains(':') {
        format!("[{host_ip}]:{port}")
    } else {
        format!("{host_ip}:{port}")
    };
    let addr: SocketAddr = host_port.parse()?;

    // Load certs.
    let client_certs =
        rustls_pemfile::certs(&mut config.cert_pem.as_bytes()).collect::<Result<Vec<_>, _>>()?;
    let client_key = rustls_pemfile::private_key(&mut config.key_pem.as_bytes())?
        .ok_or_else(|| anyhow::anyhow!("private key missing in config"))?;

    // Trusted fingerprint set.
    let trusted_set = Arc::new(Mutex::new(config.get_trusted_fingerprints_set()));
    let pending_state = Arc::new(Mutex::new(PendingPairingState::default()));

    // Custom verifier.
    let verifier = Arc::new(TofuServerVerifier::new(
        trusted_set.clone(),
        is_pairing,
        Some(pending_state.clone()),
    ));

    let mut crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(client_certs, client_key)?;
    crypto.alpn_protocols = vec![b"hyperlink".to_vec()];

    let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
        .map_err(|e| anyhow::anyhow!("failed to create QuicClientConfig: {}", e))?;
    let mut client_config = quinn::ClientConfig::new(Arc::new(quic_crypto));

    // Set transport options (keepalives, connection timeouts, migration).
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(Duration::from_secs(10).try_into()?));
    transport.keep_alive_interval(Some(Duration::from_secs(3)));
    transport.datagram_receive_buffer_size(Some(65536 * 8));
    transport.datagram_send_buffer_size(65536 * 8);
    client_config.transport_config(Arc::new(transport));

    // Create endpoint.
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse().unwrap())?;
    endpoint.set_default_client_config(client_config);

    info!("connecting to QUIC server...");
    let connection = endpoint.connect(addr, "localhost")?.await?;
    info!("QUIC connection established!");

    // The companion can't know which host it's talking to until the handshake has
    // captured the peer certificate, so it always connects with `is_pairing` set.
    // If that certificate turns out to already be pinned, this is a reconnect to a
    // paired host, not a new pairing: skip the PIN prompt and run as a normal
    // session. This is no weaker than the pinned path: the verifier records the
    // exact fingerprint presented, and it must match the trusted store byte-for-byte.
    let is_pairing = is_pairing && {
        let peer_fp = pending_state.lock().unwrap().peer_fingerprint;
        !peer_fp.is_some_and(|fp| trusted_set.lock().unwrap().contains(&fp))
    };
    if !is_pairing {
        info!("peer certificate is already trusted; continuing as a paired session");
    }

    // If pairing, check cert and calculate PIN.
    if is_pairing {
        let peer_fp = {
            let state = pending_state.lock().unwrap();
            state.peer_fingerprint
        };
        if let Some(fp) = peer_fp {
            // Save pending fingerprint.
            {
                let mut state = CLIENT_STATE.lock().unwrap();
                state.pending_pairing_fp = Some(fp);
                state.pairing_user_confirmed = None;
                state.pairing_host_accepted = false;
            }
            // Generate symmetric PIN.
            let our_cert_der = rustls_pemfile::certs(&mut config.cert_pem.as_bytes())
                .next()
                .ok_or_else(|| anyhow::anyhow!("no certs"))??;
            let our_fp = crypto::compute_fingerprint(&our_cert_der);

            let pin = crypto::generate_pairing_pin(&our_fp, &fp);
            info!("pairing PIN generated: {}", pin);
            emit_event(ClientEvent::PairingPinGenerated(pin));
        } else {
            return Err(anyhow::anyhow!(
                "peer certificate not captured during handshake"
            ));
        }
    } else {
        emit_event(ClientEvent::Connected);
    }

    // Set up control stream.
    let (mut send_stream, mut recv_stream) = connection.open_bi().await?;

    // Write stream type byte (0x50 = Control Plane) as the very first byte of the stream.
    send_stream.write_all(&[0x50]).await?;
    info!("control plane stream opened");

    // Channels for sending messages.
    let (control_tx, mut control_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    {
        let mut state = CLIENT_STATE.lock().unwrap();
        state.control_tx = Some(control_tx);
        state.connection = Some(connection.clone());
    }

    if !is_pairing {
        emit_event(ClientEvent::VideoStreamReady);
    }

    // Spawn write loop.
    let mut send_stream_clone = send_stream;
    let write_task = tokio::spawn(async move {
        while let Some(msg) = control_rx.recv().await {
            if let Err(e) = send_stream_clone.write_all(&msg).await {
                error!("control write task error: {}", e);
                break;
            }
        }
    });

    // Spawn read loop for control plane messages.
    let read_task = tokio::spawn(async move {
        let mut hdr_bytes = [0u8; hyperlink_protocol::version::HEADER_SIZE];
        loop {
            if recv_stream.read_exact(&mut hdr_bytes).await.is_err() {
                info!("control stream closed by peer or read error");
                break;
            }
            if let Ok(header) = hyperlink_protocol::version::Header::decode(&hdr_bytes) {
                let payload_len = header.payload_len as usize;
                let mut payload = vec![0u8; payload_len];
                if recv_stream.read_exact(&mut payload).await.is_err() {
                    break;
                }

                match header.message_type {
                    hyperlink_protocol::message::MessageType::NotificationActionInvoke => {
                        if let Ok(invoke) =
                            hyperlink_protocol::notification::NotificationActionInvoke::decode(
                                &payload,
                            )
                        {
                            emit_event(ClientEvent::NotificationActionInvoke {
                                key: invoke.id,
                                action_id: invoke.action_id,
                            });
                        }
                    }
                    hyperlink_protocol::message::MessageType::NotificationDismiss => {
                        if let Ok(dismiss) =
                            hyperlink_protocol::notification::NotificationDismiss::decode(&payload)
                        {
                            emit_event(ClientEvent::NotificationDismiss { key: dismiss.id });
                        }
                    }
                    hyperlink_protocol::message::MessageType::DndSync => {
                        if let Ok(dnd) = hyperlink_protocol::notification::DndSync::decode(&payload)
                        {
                            emit_event(ClientEvent::DndSync {
                                enabled: dnd.dnd_enabled,
                            });
                        }
                    }
                    hyperlink_protocol::message::MessageType::ClipboardMessage => {
                        if let Ok(clip) =
                            hyperlink_protocol::clipboard::ClipboardMessage::decode(&payload)
                        {
                            if apply_inbound_clip(&clip) {
                                emit_event(ClientEvent::ClipboardReceived {
                                    origin_id: clip.origin_id,
                                    content_type: clip.content_type as u8,
                                    mime_type: clip.mime_type,
                                    payload: clip.payload,
                                });
                            }
                        }
                    }
                    hyperlink_protocol::message::MessageType::PathProbeRequest => {
                        if let Ok(probe_req) =
                            hyperlink_protocol::resilience::PathProbeRequest::decode(&payload)
                        {
                            let state = CLIENT_STATE.lock().unwrap();
                            let mut mp = state.multipath.lock().unwrap();
                            let probe_resp = mp.handle_probe_request(&probe_req);
                            let mut resp_payload = Vec::new();
                            if probe_resp.encode(&mut resp_payload).is_ok() {
                                let resp_hdr = hyperlink_protocol::version::Header::new(
                                    hyperlink_protocol::message::MessageType::PathProbeResponse,
                                    resp_payload.len() as u32,
                                );
                                let mut pkt = Vec::with_capacity(10 + resp_payload.len());
                                if resp_hdr.encode(&mut pkt).is_ok() {
                                    pkt.extend_from_slice(&resp_payload);
                                    if let Some(ref tx) = state.control_tx {
                                        let _ = tx.send(pkt);
                                    }
                                }
                            }
                        }
                    }
                    hyperlink_protocol::message::MessageType::PathProbeResponse => {
                        if let Ok(probe_resp) =
                            hyperlink_protocol::resilience::PathProbeResponse::decode(&payload)
                        {
                            let state = CLIENT_STATE.lock().unwrap();
                            let mut mp = state.multipath.lock().unwrap();
                            mp.process_probe_response(&probe_resp);
                            let now_us = hyperlink_protocol::clock::now_us();
                            let rtt_us = now_us.saturating_sub(probe_resp.send_timestamp_us) as u32;
                            emit_event(ClientEvent::PathHealthUpdated {
                                path_id: probe_resp.path_id,
                                rtt_us,
                                active: probe_resp.path_id == mp.active_path_id,
                            });
                        }
                    }
                    hyperlink_protocol::message::MessageType::PathSwitchNotice => {
                        if let Ok(notice) =
                            hyperlink_protocol::resilience::PathSwitchNotice::decode(&payload)
                        {
                            let state = CLIENT_STATE.lock().unwrap();
                            let mut mp = state.multipath.lock().unwrap();
                            let ack = mp.handle_switch_notice(&notice);
                            let mut ack_payload = Vec::new();
                            if ack.encode(&mut ack_payload).is_ok() {
                                let ack_hdr = hyperlink_protocol::version::Header::new(
                                    hyperlink_protocol::message::MessageType::PathSwitchAck,
                                    ack_payload.len() as u32,
                                );
                                let mut pkt = Vec::with_capacity(10 + ack_payload.len());
                                if ack_hdr.encode(&mut pkt).is_ok() {
                                    pkt.extend_from_slice(&ack_payload);
                                    if let Some(ref tx) = state.control_tx {
                                        let _ = tx.send(pkt);
                                    }
                                }
                            }
                            emit_event(ClientEvent::PathFailover {
                                from_path: notice.from_path,
                                to_path: notice.to_path,
                                reason: notice.reason as u8,
                            });
                        }
                    }
                    hyperlink_protocol::message::MessageType::PathHealthReport => {
                        if let Ok(report) =
                            hyperlink_protocol::resilience::PathHealthReport::decode(&payload)
                        {
                            emit_event(ClientEvent::PathHealthUpdated {
                                path_id: report.path_id,
                                rtt_us: report.rtt_us,
                                active: report.active,
                            });
                        }
                    }
                    hyperlink_protocol::message::MessageType::HandoffPayload => {
                        if let Ok(handoff) =
                            hyperlink_protocol::handoff::HandoffPayload::decode(&payload)
                        {
                            emit_event(ClientEvent::HandoffReceived {
                                session_id: handoff.session_id,
                                handoff_type: handoff.handoff_type as u8,
                                app_id: handoff.app_id,
                                uri: handoff.uri,
                                title: handoff.title,
                                state_json: handoff.state_json,
                            });
                        }
                    }
                    hyperlink_protocol::message::MessageType::AgentConsentUpdate => {
                        if let Ok(policy) =
                            hyperlink_protocol::ambient::AgentConsentPolicy::decode(&payload)
                        {
                            emit_event(ClientEvent::AgentConsentUpdated {
                                allow_notifications: policy.allow_notifications,
                                allow_clipboard: policy.allow_clipboard,
                                allow_raw_video: policy.allow_raw_video,
                            });
                        }
                    }
                    _ => {
                        emit_event(ClientEvent::MessageReceived(
                            header.message_type as u8,
                            payload,
                        ));
                    }
                }
            }
        }
    });

    // Spawn input stream acceptor (Phase 3: host opens stream 0x40 to companion).
    let conn_clone = connection.clone();
    let input_task = tokio::spawn(async move {
        while let Ok((mut send_stream, mut recv_stream)) = conn_clone.accept_bi().await {
            note_host_accepted();
            let mut stream_type_buf = [0u8; 1];
            if recv_stream.read_exact(&mut stream_type_buf).await.is_err() {
                continue;
            }
            let stream_type = stream_type_buf[0];
            if stream_type == 0x40 {
                info!("accepted input stream 0x40 from host");
                // Its own task: handling it inline would keep this loop from
                // accepting the host's other streams (clipboard, files) for as
                // long as the input stream stays open, i.e. forever.
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1024];
                    let mut ack_packet = Vec::with_capacity(
                        hyperlink_protocol::version::HEADER_SIZE
                            + hyperlink_protocol::input::INPUT_ACK_SIZE,
                    );
                    let mut seq: u32 = 0;

                    loop {
                        let mut hdr_bytes = [0u8; hyperlink_protocol::version::HEADER_SIZE];
                        if recv_stream.read_exact(&mut hdr_bytes).await.is_err() {
                            break;
                        }
                        if let Ok(header) = hyperlink_protocol::version::Header::decode(&hdr_bytes)
                        {
                            let payload_len = header.payload_len as usize;
                            if payload_len > buf.len() {
                                buf.resize(payload_len, 0);
                            }
                            if recv_stream
                                .read_exact(&mut buf[..payload_len])
                                .await
                                .is_err()
                            {
                                break;
                            }

                            match header.message_type {
                                hyperlink_protocol::message::MessageType::PointerEvent => {
                                    if let Ok(p) = hyperlink_protocol::input::PointerEvent::decode(
                                        &buf[..payload_len],
                                    ) {
                                        emit_event(ClientEvent::PointerEvent {
                                            action: p.action as u8,
                                            button: p.button as u8,
                                            x_norm: p.x_norm,
                                            y_norm: p.y_norm,
                                            pressure: p.pressure,
                                        });
                                    }
                                }
                                hyperlink_protocol::message::MessageType::KeyEvent => {
                                    if let Ok(k) =
                                        hyperlink_protocol::input::KeyEvent::decode(&buf[..payload_len])
                                    {
                                        emit_event(ClientEvent::KeyEvent {
                                            action: k.action as u8,
                                            keycode: k.keycode,
                                            modifiers: k.modifiers,
                                        });
                                    }
                                }
                                hyperlink_protocol::message::MessageType::ScrollEvent => {
                                    if let Ok(s) = hyperlink_protocol::input::ScrollEvent::decode(
                                        &buf[..payload_len],
                                    ) {
                                        emit_event(ClientEvent::ScrollEvent {
                                            dx: s.dx,
                                            dy: s.dy,
                                            x_norm: s.x_norm,
                                            y_norm: s.y_norm,
                                        });
                                    }
                                }
                                hyperlink_protocol::message::MessageType::KeyframeRequest => {
                                    emit_event(ClientEvent::KeyframeRequest);
                                }
                                hyperlink_protocol::message::MessageType::NavEvent => {
                                    if let Ok(n) =
                                        hyperlink_protocol::input::NavEvent::decode(&buf[..payload_len])
                                    {
                                        emit_event(ClientEvent::NavEvent {
                                            action: n.action as u8,
                                        });
                                    }
                                }
                                hyperlink_protocol::message::MessageType::NotificationActionInvoke => {
                                    if let Ok(invoke) =
                                        hyperlink_protocol::notification::NotificationActionInvoke::decode(
                                            &buf[..payload_len],
                                        )
                                    {
                                        emit_event(ClientEvent::NotificationActionInvoke {
                                            key: invoke.id,
                                            action_id: invoke.action_id,
                                        });
                                    }
                                }
                                hyperlink_protocol::message::MessageType::NotificationDismiss => {
                                    if let Ok(dismiss) =
                                        hyperlink_protocol::notification::NotificationDismiss::decode(
                                            &buf[..payload_len],
                                        )
                                    {
                                        emit_event(ClientEvent::NotificationDismiss {
                                            key: dismiss.id,
                                        });
                                    }
                                }
                                hyperlink_protocol::message::MessageType::DndSync => {
                                    if let Ok(dnd) =
                                        hyperlink_protocol::notification::DndSync::decode(
                                            &buf[..payload_len],
                                        )
                                    {
                                        emit_event(ClientEvent::DndSync {
                                            enabled: dnd.dnd_enabled,
                                        });
                                    }
                                }
                                _ => {}
                            }

                            // Send back InputAck for RTT measurement
                            seq = seq.wrapping_add(1);
                            let now_us = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_micros() as u64)
                                .unwrap_or(0);
                            let ack = hyperlink_protocol::input::InputAck {
                                seq,
                                timestamp_us: now_us,
                            };
                            let mut ack_payload = Vec::new();
                            if ack.encode(&mut ack_payload).is_ok() {
                                let ack_header = hyperlink_protocol::version::Header::new(
                                    hyperlink_protocol::message::MessageType::InputAck,
                                    ack_payload.len() as u32,
                                );
                                ack_packet.clear();
                                if ack_header.encode(&mut ack_packet).is_ok() {
                                    ack_packet.extend_from_slice(&ack_payload);
                                    if send_stream.write_all(&ack_packet).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                });
            } else if stream_type == 0x70 {
                info!("accepted dedicated clipboard stream 0x70 from host");
                tokio::spawn(async move {
                    loop {
                        let mut hdr_bytes = [0u8; hyperlink_protocol::version::HEADER_SIZE];
                        if recv_stream.read_exact(&mut hdr_bytes).await.is_err() {
                            break;
                        }
                        if let Ok(header) = hyperlink_protocol::version::Header::decode(&hdr_bytes)
                        {
                            let payload_len = header.payload_len as usize;
                            let mut payload = vec![0u8; payload_len];
                            if recv_stream.read_exact(&mut payload).await.is_err() {
                                break;
                            }
                            if header.message_type
                                == hyperlink_protocol::message::MessageType::ClipboardMessage
                            {
                                if let Ok(clip) =
                                    hyperlink_protocol::clipboard::ClipboardMessage::decode(
                                        &payload,
                                    )
                                {
                                    let now_us = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_micros() as u64)
                                        .unwrap_or(0);
                                    let ack = hyperlink_protocol::clipboard::ClipboardAck {
                                        seq: clip.seq,
                                        content_hash: clip.content_hash,
                                        origin_id: clip.origin_id.clone(),
                                        host_received_us: now_us,
                                    };
                                    let mut ack_payload = Vec::new();
                                    if ack.encode(&mut ack_payload).is_ok() {
                                        let ack_hdr = hyperlink_protocol::version::Header::new(
                                            hyperlink_protocol::message::MessageType::ClipboardAck,
                                            ack_payload.len() as u32,
                                        );
                                        let mut ack_packet =
                                            Vec::with_capacity(10 + ack_payload.len());
                                        if ack_hdr.encode(&mut ack_packet).is_ok() {
                                            ack_packet.extend_from_slice(&ack_payload);
                                            let _ = send_stream.write_all(&ack_packet).await;
                                        }
                                    }

                                    if apply_inbound_clip(&clip) {
                                        emit_event(ClientEvent::ClipboardReceived {
                                            origin_id: clip.origin_id,
                                            content_type: clip.content_type as u8,
                                            mime_type: clip.mime_type,
                                            payload: clip.payload,
                                        });
                                    }
                                }
                            }
                        }
                    }
                });
            } else if stream_type == 0x80 {
                info!("accepted dedicated file access stream 0x80 from host");
                tokio::spawn(async move {
                    loop {
                        let mut hdr_bytes = [0u8; hyperlink_protocol::version::HEADER_SIZE];
                        if recv_stream.read_exact(&mut hdr_bytes).await.is_err() {
                            break;
                        }
                        if let Ok(header) = hyperlink_protocol::version::Header::decode(&hdr_bytes)
                        {
                            let payload_len = header.payload_len as usize;
                            let mut payload = vec![0u8; payload_len];
                            if recv_stream.read_exact(&mut payload).await.is_err() {
                                break;
                            }
                            match header.message_type {
                                hyperlink_protocol::message::MessageType::FileStatRequest => {
                                    if let Ok(req) =
                                        hyperlink_protocol::file_access::FileStatRequest::decode(
                                            &payload,
                                        )
                                    {
                                        let resp = file_provider::handle_file_stat(&req);
                                        let mut resp_payload = Vec::new();
                                        if resp.encode(&mut resp_payload).is_ok() {
                                            let resp_hdr = hyperlink_protocol::version::Header::new(
                                                hyperlink_protocol::message::MessageType::FileStatResponse,
                                                resp_payload.len() as u32,
                                            );
                                            let mut packet =
                                                Vec::with_capacity(10 + resp_payload.len());
                                            if resp_hdr.encode(&mut packet).is_ok() {
                                                packet.extend_from_slice(&resp_payload);
                                                if send_stream.write_all(&packet).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                hyperlink_protocol::message::MessageType::FileListRequest => {
                                    if let Ok(req) =
                                        hyperlink_protocol::file_access::FileListRequest::decode(
                                            &payload,
                                        )
                                    {
                                        let resp = file_provider::handle_file_list(&req);
                                        let mut resp_payload = Vec::new();
                                        if resp.encode(&mut resp_payload).is_ok() {
                                            let resp_hdr = hyperlink_protocol::version::Header::new(
                                                hyperlink_protocol::message::MessageType::FileListResponse,
                                                resp_payload.len() as u32,
                                            );
                                            let mut packet =
                                                Vec::with_capacity(10 + resp_payload.len());
                                            if resp_hdr.encode(&mut packet).is_ok() {
                                                packet.extend_from_slice(&resp_payload);
                                                if send_stream.write_all(&packet).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                hyperlink_protocol::message::MessageType::FileReadChunkRequest => {
                                    if let Ok(req) =
                                        hyperlink_protocol::file_access::FileReadChunkRequest::decode(
                                            &payload,
                                        )
                                    {
                                        let resp = file_provider::handle_file_read_chunk(&req);
                                        let mut resp_payload = Vec::new();
                                        if resp.encode(&mut resp_payload).is_ok() {
                                            let resp_hdr = hyperlink_protocol::version::Header::new(
                                                hyperlink_protocol::message::MessageType::FileReadChunkResponse,
                                                resp_payload.len() as u32,
                                            );
                                            let mut packet =
                                                Vec::with_capacity(10 + resp_payload.len());
                                            if resp_hdr.encode(&mut packet).is_ok() {
                                                packet.extend_from_slice(&resp_payload);
                                                if send_stream.write_all(&packet).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                hyperlink_protocol::message::MessageType::FileWriteChunkRequest => {
                                    if let Ok(req) =
                                        hyperlink_protocol::file_access::FileWriteChunkRequest::decode(
                                            &payload,
                                        )
                                    {
                                        let resp = file_provider::handle_file_write_chunk(&req);
                                        let mut resp_payload = Vec::new();
                                        if resp.encode(&mut resp_payload).is_ok() {
                                            let resp_hdr = hyperlink_protocol::version::Header::new(
                                                hyperlink_protocol::message::MessageType::FileWriteChunkResponse,
                                                resp_payload.len() as u32,
                                            );
                                            let mut packet =
                                                Vec::with_capacity(10 + resp_payload.len());
                                            if resp_hdr.encode(&mut packet).is_ok() {
                                                packet.extend_from_slice(&resp_payload);
                                                if send_stream.write_all(&packet).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                hyperlink_protocol::message::MessageType::FileThumbnailRequest => {
                                    if let Ok(req) =
                                        hyperlink_protocol::file_access::FileThumbnailRequest::decode(
                                            &payload,
                                        )
                                    {
                                        let resp = file_provider::handle_file_thumbnail(&req);
                                        let mut resp_payload = Vec::new();
                                        if resp.encode(&mut resp_payload).is_ok() {
                                            let resp_hdr = hyperlink_protocol::version::Header::new(
                                                hyperlink_protocol::message::MessageType::FileThumbnailResponse,
                                                resp_payload.len() as u32,
                                            );
                                            let mut packet =
                                                Vec::with_capacity(10 + resp_payload.len());
                                            if resp_hdr.encode(&mut packet).is_ok() {
                                                packet.extend_from_slice(&resp_payload);
                                                if send_stream.write_all(&packet).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                });
            }
        }
    });

    // Wait until connection closes or tasks finish.
    let reason = tokio::select! {
        err = connection.closed() => {
            info!("QUIC connection closed: {}", err);
            close_reason_code(&err)
        }
        _ = write_task => "connection_lost".to_string(),
        _ = read_task => "connection_lost".to_string(),
        _ = input_task => "connection_lost".to_string(),
    };

    // A pairing that didn't finish leaves nothing behind.
    {
        let mut state = CLIENT_STATE.lock().unwrap();
        state.pending_pairing_fp = None;
        state.pairing_user_confirmed = None;
        state.pairing_host_accepted = false;
    }
    emit_event(ClientEvent::Disconnected(reason));

    // Reset control channel.
    {
        let mut state = CLIENT_STATE.lock().unwrap();
        state.control_tx = None;
    }

    Ok(())
}

/// Send an encoded video frame over QUIC datagrams, fragmenting it if necessary.
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_sendVideoFrame(
    env: JNIEnv,
    _class: JClass,
    frame_data: JByteArray,
    frame_id: jint,
    timestamp_us: jlong,
    is_keyframe: jboolean,
    width: jint,
    height: jint,
) -> jboolean {
    let frame_bytes = match env.convert_byte_array(&frame_data) {
        Ok(b) => b,
        Err(_) => return 0,
    };
    let frame_id = frame_id as u32;
    let timestamp_us = timestamp_us as u64;
    let is_keyframe = is_keyframe != 0;
    let width = width as u16;
    let height = height as u16;

    let state = CLIENT_STATE.lock().unwrap();
    let conn = match &state.connection {
        Some(c) => c.clone(),
        None => return 0,
    };

    // Fragment large video frame payloads to fit inside MTU datagram limit.
    let max_fragment_size = 1000;
    let total_len = frame_bytes.len();
    let fragment_count = total_len.div_ceil(max_fragment_size).max(1);

    for fragment_idx in 0..fragment_count {
        let start = fragment_idx * max_fragment_size;
        let end = (start + max_fragment_size).min(total_len);
        let fragment_payload = &frame_bytes[start..end];

        let video_header = hyperlink_protocol::video::VideoFrameHeader {
            frame_id,
            timestamp_us,
            is_keyframe,
            width,
            height,
            payload_len: fragment_payload.len() as u32,
            fragment_idx: fragment_idx as u16,
            fragment_count: fragment_count as u16,
        };

        // Serialize video header + fragment data.
        let mut video_buf = Vec::new();
        if video_header.encode(&mut video_buf).is_err() {
            return 0;
        }
        video_buf.extend_from_slice(fragment_payload);

        // Serialize HyperLink Packet Header.
        let hl_header = hyperlink_protocol::version::Header::new(
            hyperlink_protocol::message::MessageType::VideoFrame,
            video_buf.len() as u32,
        );
        let mut packet_buf = hl_header.to_bytes().to_vec();
        packet_buf.extend_from_slice(&video_buf);

        // Send as QUIC datagram
        if let Err(e) = conn.send_datagram(bytes::Bytes::from(packet_buf)) {
            warn!("failed to send video frame datagram fragment: {}", e);
            return 0;
        }
    }

    1 // true
}

/// Send video configuration (SPS/PPS) over a reliable unidirectional stream.
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_sendVideoConfig(
    env: JNIEnv,
    _class: JClass,
    sps: JByteArray,
    pps: JByteArray,
    bitrate: jint,
    fps: jint,
) -> jboolean {
    let sps_bytes = match env.convert_byte_array(&sps) {
        Ok(b) => b,
        Err(_) => return 0,
    };
    let pps_bytes = match env.convert_byte_array(&pps) {
        Ok(b) => b,
        Err(_) => return 0,
    };
    let bitrate_bps = bitrate as u32;
    let fps = fps as u8;

    let state = CLIENT_STATE.lock().unwrap();
    let conn = match &state.connection {
        Some(c) => c.clone(),
        None => return 0,
    };

    // Serialize VideoConfig.
    let config = hyperlink_protocol::video::VideoConfig {
        sps: sps_bytes,
        pps: pps_bytes,
        bitrate_bps,
        fps,
    };
    let mut config_buf = Vec::new();
    if config.encode(&mut config_buf).is_err() {
        return 0;
    }

    // Serialize HyperLink Packet Header.
    let hl_header = hyperlink_protocol::version::Header::new(
        hyperlink_protocol::message::MessageType::VideoConfig,
        config_buf.len() as u32,
    );
    let mut packet_buf = hl_header.to_bytes().to_vec();
    packet_buf.extend_from_slice(&config_buf);

    // Spawn async task to open stream and transmit.
    RUNTIME.spawn(async move {
        match conn.open_uni().await {
            Ok(mut send_stream) => {
                // Write the stream type byte (0x31 = Video Config) first.
                if let Err(e) = send_stream.write_all(&[0x31]).await {
                    warn!("failed to write video config stream type: {}", e);
                    return;
                }
                // Write the header + payload.
                if let Err(e) = send_stream.write_all(&packet_buf).await {
                    warn!("failed to write video config payload: {}", e);
                    return;
                }
                let _ = send_stream.finish();
                info!("successfully sent video config stream");
            }
            Err(e) => {
                warn!(
                    "failed to open unidirectional stream for video config: {}",
                    e
                );
            }
        }
    });

    1 // true
}

/// Register an active network interface with the multipath manager (Phase 7).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_NetworkMonitorService_onNetworkAvailable(
    mut env: JNIEnv,
    _class: JClass,
    path_id: jint,
    ip: JString,
    port: jint,
) {
    let ip_str: String = match env.get_string(&ip) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    let addr_str = format!("{}:{}", ip_str, port);
    if let Ok(sock_addr) = addr_str.parse::<SocketAddr>() {
        let state = CLIENT_STATE.lock().unwrap();
        let mut mp = state.multipath.lock().unwrap();
        mp.register_interface(path_id as u8, sock_addr);
    }
}

/// Mark a network interface as lost (Phase 7).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_NetworkMonitorService_onNetworkLost(
    _env: JNIEnv,
    _class: JClass,
    path_id: jint,
) {
    let state = CLIENT_STATE.lock().unwrap();
    let mut mp = state.multipath.lock().unwrap();
    mp.mark_interface_lost(path_id as u8);
}

/// Trigger a manual or policy-based failover (Phase 7).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_NetworkMonitorService_triggerFailover(
    _env: JNIEnv,
    _class: JClass,
    target_path_id: jint,
    reason_code: jint,
) -> jboolean {
    let state = CLIENT_STATE.lock().unwrap();
    let mut mp = state.multipath.lock().unwrap();
    let reason = hyperlink_protocol::resilience::FailoverReason::from(reason_code as u8);
    if let Some(notice) = mp.initiate_failover(target_path_id as u8, reason) {
        let mut payload = Vec::new();
        if notice.encode(&mut payload).is_ok() {
            let header = hyperlink_protocol::version::Header::new(
                hyperlink_protocol::message::MessageType::PathSwitchNotice,
                payload.len() as u32,
            );
            let mut packet = Vec::with_capacity(10 + payload.len());
            if header.encode(&mut packet).is_ok() {
                packet.extend_from_slice(&payload);
                if let Some(ref tx) = state.control_tx {
                    if tx.send(packet).is_ok() {
                        return 1;
                    }
                }
            }
        }
    }
    0
}

/// Get the currently active path ID (Phase 7).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_NetworkMonitorService_getActivePath(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    let state = CLIENT_STATE.lock().unwrap();
    let mp = state.multipath.lock().unwrap();
    mp.active_path_id as jint
}

/// Returns this device's own certificate fingerprint (colon-separated hex), or
/// `null` if the client hasn't been initialized yet. `ProximityRangingService`
/// needs this to broadcast in its beacons — `handle_proximity_beacon` on the host
/// side matches the beacon's fingerprint prefix against *this* device's pinned
/// fingerprint in its trusted-peers set, not the host's own (Phase 8).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_getOwnFingerprint(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let config = {
        let state = CLIENT_STATE.lock().unwrap();
        state.config.clone()
    };
    let Some(config) = config else {
        return std::ptr::null_mut();
    };
    let Ok(certs) =
        rustls_pemfile::certs(&mut config.cert_pem.as_bytes()).collect::<Result<Vec<_>, _>>()
    else {
        return std::ptr::null_mut();
    };
    let Some(our_cert) = certs.first() else {
        return std::ptr::null_mut();
    };
    let fp = hyperlink_protocol::crypto::compute_fingerprint(our_cert);
    let fp_str = hyperlink_protocol::crypto::fingerprint_to_string(&fp);
    match env.new_string(fp_str) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Send proximity ranging beacon to host (Phase 8).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_sendProximityBeacon(
    mut env: JNIEnv,
    _class: JClass,
    cert_fp: JString,
    technology: jint,
    distance_cm: jint,
    rssi_dbm: jint,
    confidence_pct: jint,
    nonce: jlong,
) -> jboolean {
    let fp_str: String = match env.get_string(&cert_fp) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };

    if let Ok(beacon) = proximity::create_proximity_beacon(
        &fp_str,
        technology as u8,
        distance_cm as u16,
        rssi_dbm as i8,
        confidence_pct as u8,
        nonce as u64,
    ) {
        let mut payload = Vec::new();
        if beacon.encode(&mut payload).is_ok() {
            let header = hyperlink_protocol::version::Header::new(
                hyperlink_protocol::message::MessageType::ProximityBeacon,
                payload.len() as u32,
            );
            let mut packet = Vec::with_capacity(10 + payload.len());
            if header.encode(&mut packet).is_ok() {
                packet.extend_from_slice(&payload);
                let state = CLIENT_STATE.lock().unwrap();
                if let Some(ref tx) = state.control_tx {
                    if tx.send(packet).is_ok() {
                        return 1;
                    }
                }
            }
        }
    }
    0
}

/// Save current companion workflow state on host (Phase 8).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_saveWorkflowState(
    mut env: JNIEnv,
    _class: JClass,
    width: jint,
    height: jint,
    package_name: JString,
    orientation: jint,
) -> jboolean {
    let pkg_str: String = match env.get_string(&package_name) {
        Ok(s) => s.into(),
        Err(_) => "com.android.launcher".to_string(),
    };

    let wf =
        proximity::capture_workflow_state(width as u32, height as u32, pkg_str, orientation as u8);

    let mut payload = Vec::new();
    if wf.encode(&mut payload).is_ok() {
        let header = hyperlink_protocol::version::Header::new(
            hyperlink_protocol::message::MessageType::WorkflowStateSave,
            payload.len() as u32,
        );
        let mut packet = Vec::with_capacity(10 + payload.len());
        if header.encode(&mut packet).is_ok() {
            packet.extend_from_slice(&payload);
            let state = CLIENT_STATE.lock().unwrap();
            if let Some(ref tx) = state.control_tx {
                if tx.send(packet).is_ok() {
                    return 1;
                }
            }
        }
    }
    0
}

/// Request host to restore saved workflow state (Phase 8).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_restoreWorkflowState(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let header = hyperlink_protocol::version::Header::new(
        hyperlink_protocol::message::MessageType::WorkflowStateRestore,
        0,
    );
    let mut packet = Vec::with_capacity(10);
    if header.encode(&mut packet).is_ok() {
        let state = CLIENT_STATE.lock().unwrap();
        if let Some(ref tx) = state.control_tx {
            if tx.send(packet).is_ok() {
                return 1;
            }
        }
    }
    0
}

/// Send state handoff payload to peer (Phase 9).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_sendHandoff(
    mut env: JNIEnv,
    _class: JClass,
    session_id: jlong,
    handoff_type: jint,
    source_origin: JString,
    app_id: JString,
    uri: JString,
    title: JString,
    state_json: JString,
) -> jboolean {
    let origin_str: String = match env.get_string(&source_origin) {
        Ok(s) => s.into(),
        Err(_) => "phone:AndroidCompanion".to_string(),
    };
    let app_str: String = match env.get_string(&app_id) {
        Ok(s) => s.into(),
        Err(_) => "".to_string(),
    };
    let uri_str: String = match env.get_string(&uri) {
        Ok(s) => s.into(),
        Err(_) => "".to_string(),
    };
    let title_str: String = match env.get_string(&title) {
        Ok(s) => s.into(),
        Err(_) => "".to_string(),
    };
    let json_str: String = match env.get_string(&state_json) {
        Ok(s) => s.into(),
        Err(_) => "{}".to_string(),
    };

    let h_type = match hyperlink_protocol::handoff::HandoffType::try_from(handoff_type as u8) {
        Ok(t) => t,
        Err(_) => return 0,
    };

    let payload = hyperlink_protocol::handoff::HandoffPayload {
        session_id: session_id as u64,
        source_origin: origin_str,
        handoff_type: h_type,
        app_id: app_str,
        uri: uri_str,
        title: title_str,
        state_json: json_str,
        timestamp_us: hyperlink_protocol::clock::now_us(),
    };

    let mut buf = Vec::new();
    if payload.encode(&mut buf).is_ok() {
        let header = hyperlink_protocol::version::Header::new(
            hyperlink_protocol::message::MessageType::HandoffPayload,
            buf.len() as u32,
        );
        let mut packet = Vec::with_capacity(10 + buf.len());
        if header.encode(&mut packet).is_ok() {
            packet.extend_from_slice(&buf);
            let state = CLIENT_STATE.lock().unwrap();
            if let Some(ref tx) = state.control_tx {
                if tx.send(packet).is_ok() {
                    return 1;
                }
            }
        }
    }
    0
}

/// Acknowledge state handoff (Phase 9).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_acknowledgeHandoff(
    _env: JNIEnv,
    _class: JClass,
    session_id: jlong,
    accepted: jboolean,
    status_code: jint,
) -> jboolean {
    if let Ok(ack_bytes) =
        handoff::create_handoff_ack(session_id as u64, accepted != 0, status_code as u8)
    {
        let header = hyperlink_protocol::version::Header::new(
            hyperlink_protocol::message::MessageType::HandoffAck,
            ack_bytes.len() as u32,
        );
        let mut packet = Vec::with_capacity(10 + ack_bytes.len());
        if header.encode(&mut packet).is_ok() {
            packet.extend_from_slice(&ack_bytes);
            let state = CLIENT_STATE.lock().unwrap();
            if let Some(ref tx) = state.control_tx {
                if tx.send(packet).is_ok() {
                    return 1;
                }
            }
        }
    }
    0
}

/// Publish an ambient context telemetry event to Linux host (Phase 10).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_publishAmbientEvent(
    mut env: JNIEnv,
    _class: JClass,
    event_id: jlong,
    category: jint,
    source: JString,
    summary: JString,
    metadata_json: JString,
) -> jboolean {
    let source_str: String = match env.get_string(&source) {
        Ok(s) => s.into(),
        Err(_) => "phone:AndroidCompanion".to_string(),
    };
    let summary_str: String = match env.get_string(&summary) {
        Ok(s) => s.into(),
        Err(_) => "Telemetry event".to_string(),
    };
    let meta_str: String = match env.get_string(&metadata_json) {
        Ok(s) => s.into(),
        Err(_) => "{}".to_string(),
    };

    if let Ok(event) = ambient::create_ambient_event(
        event_id as u64,
        category as u8,
        &source_str,
        &summary_str,
        &meta_str,
    ) {
        let mut payload = Vec::new();
        if event.encode(&mut payload).is_ok() {
            let header = hyperlink_protocol::version::Header::new(
                hyperlink_protocol::message::MessageType::AmbientEventPublish,
                payload.len() as u32,
            );
            let mut packet = Vec::with_capacity(10 + payload.len());
            if header.encode(&mut packet).is_ok() {
                packet.extend_from_slice(&payload);
                let state = CLIENT_STATE.lock().unwrap();
                if let Some(ref tx) = state.control_tx {
                    if tx.send(packet).is_ok() {
                        return 1;
                    }
                }
            }
        }
    }
    0
}

/// Update agent consent policy from phone (Phase 10).
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_updateAgentConsent(
    _env: JNIEnv,
    _class: JClass,
    allow_notifications: jboolean,
    allow_screen_state: jboolean,
    allow_foreground_app: jboolean,
    allow_device_status: jboolean,
    allow_clipboard: jboolean,
    allow_media_state: jboolean,
    allow_raw_video: jboolean,
) -> jboolean {
    let policy = hyperlink_protocol::ambient::AgentConsentPolicy {
        allow_notifications: allow_notifications != 0,
        allow_screen_state: allow_screen_state != 0,
        allow_foreground_app: allow_foreground_app != 0,
        allow_device_status: allow_device_status != 0,
        allow_clipboard: allow_clipboard != 0,
        allow_media_state: allow_media_state != 0,
        allow_raw_video: allow_raw_video != 0,
    };

    let mut payload = Vec::new();
    policy.encode(&mut payload);

    let header = hyperlink_protocol::version::Header::new(
        hyperlink_protocol::message::MessageType::AgentConsentUpdate,
        payload.len() as u32,
    );
    let mut packet = Vec::with_capacity(10 + payload.len());
    if header.encode(&mut packet).is_ok() {
        packet.extend_from_slice(&payload);
        let state = CLIENT_STATE.lock().unwrap();
        if let Some(ref tx) = state.control_tx {
            if tx.send(packet).is_ok() {
                return 1;
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_android_clipboard_loop_and_recency_prevention() {
        // Reset state for test
        {
            let mut state = CLIENT_STATE.lock().unwrap();
            state.recent_clipboard_hashes.clear();
            state.last_local_clip_write_us = 1000;
            state.last_applied_remote_clip_us = 0;
            state.last_applied_remote_clip_seq = 0;
        }

        // 1. Stale clip (timestamp older than local clip write) should be rejected
        let stale_clip = hyperlink_protocol::clipboard::ClipboardMessage::new_text(
            "host:desktop".to_string(),
            "Stale clip",
            1,
            500, // Older than 1000
        );
        assert!(!apply_inbound_clip(&stale_clip));

        // 2. Fresh clip should be accepted
        let fresh_clip = hyperlink_protocol::clipboard::ClipboardMessage::new_text(
            "host:desktop".to_string(),
            "Fresh clip",
            2,
            2000, // Newer than 1000
        );
        assert!(apply_inbound_clip(&fresh_clip));

        // 3. Replay / duplicate echo of the same clip should be rejected
        assert!(!apply_inbound_clip(&fresh_clip));

        // 4. Out of order clip (timestamp older than already applied remote clip) should be rejected
        let out_of_order = hyperlink_protocol::clipboard::ClipboardMessage::new_text(
            "host:desktop".to_string(),
            "Out of order clip",
            3,
            1500, // Older than 2000
        );
        assert!(!apply_inbound_clip(&out_of_order));
    }
}
