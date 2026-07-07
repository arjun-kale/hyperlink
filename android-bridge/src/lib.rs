//! JNI Bridge for HyperLink Android Companion.
//!
//! Handles background QUIC client connections, TOFU certificate verification,
//! pairing PIN generation, mDNS discovery integration, and event polling.

#![allow(clippy::missing_safety_doc)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jint, jstring};
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
    /// Connection closed or failed.
    Disconnected(String),
    /// Heartbeat or message received.
    MessageReceived(u8, Vec<u8>),
}

/// Global mutable state for the client.
#[derive(Default)]
struct ClientState {
    config: Option<DeviceConfig>,
    config_path: Option<PathBuf>,
    pending_pairing_fp: Option<[u8; 32]>,
    event_rx: Option<mpsc::UnboundedReceiver<ClientEvent>>,
    event_tx: Option<mpsc::UnboundedSender<ClientEvent>>,
    control_tx: Option<mpsc::UnboundedSender<Vec<u8>>>,
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

/// Confirm pairing and persist host fingerprint.
#[no_mangle]
pub unsafe extern "system" fn Java_com_hyperlink_companion_QuicClient_confirmPairing(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let mut state = CLIENT_STATE.lock().unwrap();

    if let (Some(fp), Some(mut config), Some(path)) = (
        state.pending_pairing_fp.take(),
        state.config.clone(),
        state.config_path.as_ref(),
    ) {
        let fp_str = crypto::fingerprint_to_string(&fp);
        info!(
            "pairing confirmed: adding trusted host fingerprint: {}",
            fp_str
        );

        config.add_trusted_peer("Linux-Host", &fp_str);
        if let Err(e) = config.save(path) {
            error!("failed to save updated config: {}", e);
            return 0; // false
        }

        state.config = Some(config);
        1 // true
    } else {
        warn!("cannot confirm pairing: no pending fingerprint or config missing");
        0 // false
    }
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
                        format!("{{\"type\":\"disconnected\",\"reason\":\"{}\"}}", reason)
                    }
                    ClientEvent::MessageReceived(stream_type, data) => {
                        let hex: String = data.iter().map(|b| format!("{:02x}", b)).collect();
                        format!(
                            "{{\"type\":\"message\",\"stream_type\":{},\"payload\":\"{}\"}}",
                            stream_type, hex
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
    let addr: SocketAddr = format!("{}:{}", host_ip, port).parse()?;

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

    // Set transport options (keepalives, connection timeouts).
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(Duration::from_secs(10).try_into()?));
    transport.keep_alive_interval(Some(Duration::from_secs(3)));
    client_config.transport_config(Arc::new(transport));

    // Create endpoint.
    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse().unwrap())?;
    endpoint.set_default_client_config(client_config);

    info!("connecting to QUIC server...");
    let connection = endpoint.connect(addr, "localhost")?.await?;
    info!("QUIC connection established!");

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

    // Spawn read loop.
    let read_task = tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        loop {
            match recv_stream.read(&mut buf).await {
                Ok(Some(n)) => {
                    // Emit message received event.
                    emit_event(ClientEvent::MessageReceived(0x50, buf[..n].to_vec()));
                }
                Ok(None) => {
                    info!("control stream closed by peer");
                    break;
                }
                Err(e) => {
                    error!("control read task error: {}", e);
                    break;
                }
            }
        }
    });

    // Wait until connection closes or tasks finish.
    tokio::select! {
        _ = connection.closed() => {
            info!("QUIC connection closed by peer");
        }
        _ = write_task => {}
        _ = read_task => {}
    }

    emit_event(ClientEvent::Disconnected("Connection closed".to_string()));

    // Reset control channel.
    {
        let mut state = CLIENT_STATE.lock().unwrap();
        state.control_tx = None;
    }

    Ok(())
}
