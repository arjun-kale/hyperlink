//! Quinn server configuration, mutual TLS (mTLS) verifications, and client session loops.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use quinn::Endpoint;
use rustls::pki_types::CertificateDer;
use tracing::{debug, error, info, warn};

use hyperlink_protocol::config::DeviceConfig;
use hyperlink_protocol::crypto::{self, PendingPairingState, TofuClientVerifier};

/// Minimum spacing between keyframe requests to the phone after frame loss.
const KEYFRAME_REQUEST_INTERVAL: Duration = Duration::from_millis(250);

/// How long an unanswered pairing request waits for the user before it's
/// treated as a rejection.
const PAIRING_DECISION_TIMEOUT: Duration = Duration::from_secs(120);

/// Live server state the GUI needs to reach into: the pairing switch the
/// client verifier follows, the in-memory trust set, and the sessions that are
/// currently up (so revoking a device can drop it immediately).
struct ServerHandles {
    pairing_open: Arc<AtomicBool>,
    #[cfg_attr(not(feature = "video"), allow(dead_code))]
    trusted: Arc<Mutex<HashSet<[u8; 32]>>>,
    sessions: Mutex<Vec<([u8; 32], quinn::Connection)>>,
}

static SERVER: OnceLock<ServerHandles> = OnceLock::new();

/// Opens or closes the window during which an unknown phone may pair. While
/// closed, unknown certificates are rejected during the TLS handshake itself.
pub fn set_pairing_open(open: bool) {
    if let Some(server) = SERVER.get() {
        server.pairing_open.store(open, Ordering::SeqCst);
        info!(open, "pairing window changed");
    }
}

/// Removes a device from the live trust set and drops its session if it's
/// connected. The caller is responsible for persisting the config change.
#[cfg_attr(not(feature = "video"), allow(dead_code))]
pub fn revoke_device(fingerprint: &str) {
    let Some(server) = SERVER.get() else { return };
    let Ok(fp) = crypto::string_to_fingerprint(fingerprint) else {
        return;
    };
    server.trusted.lock().unwrap().remove(&fp);
    for (session_fp, conn) in server.sessions.lock().unwrap().iter() {
        if *session_fp == fp {
            conn.close(0u32.into(), b"device revoked");
            info!("closed session for revoked device");
        }
    }
}

/// Sends a session event to the GUI, if one is running.
fn notify_gui(event: crate::SessionEvent) {
    #[cfg(feature = "video")]
    if let Some(tx) = crate::UI_SENDER.get() {
        let _ = tx.try_send(crate::VideoGuiMessage::Session(event));
    }
    #[cfg(not(feature = "video"))]
    let _ = event;
}

/// Tells the GUI a session ended, however `handle_incoming_connection` exits.
struct SessionGuard {
    fingerprint: [u8; 32],
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if let Some(server) = SERVER.get() {
            server
                .sessions
                .lock()
                .unwrap()
                .retain(|(fp, _)| *fp != self.fingerprint);
        }
        notify_gui(crate::SessionEvent::PhoneDisconnected);
    }
}

/// Starts the QUIC server and listens for incoming connections.
pub async fn start_server(
    bind_addr: SocketAddr,
    config: DeviceConfig,
    config_path: std::path::PathBuf,
    is_pairing: bool,
) -> anyhow::Result<()> {
    // Parse certificate and key.
    let certs =
        rustls_pemfile::certs(&mut config.cert_pem.as_bytes()).collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut config.key_pem.as_bytes())?
        .ok_or_else(|| anyhow::anyhow!("private key missing in host config"))?;

    let trusted_set = Arc::new(Mutex::new(config.get_trusted_fingerprints_set()));
    let pending_state = Arc::new(Mutex::new(PendingPairingState::default()));
    let pairing_open = Arc::new(AtomicBool::new(is_pairing));

    let _ = SERVER.set(ServerHandles {
        pairing_open: pairing_open.clone(),
        trusted: trusted_set.clone(),
        sessions: Mutex::new(Vec::new()),
    });

    // Custom client verifier. It follows `pairing_open` live, so the GUI can open
    // a pairing window without restarting the server.
    let verifier = Arc::new(TofuClientVerifier::with_pairing_switch(
        trusted_set.clone(),
        pairing_open.clone(),
        Some(pending_state.clone()),
    ));

    // Configure server crypto.
    let mut server_crypto = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs.clone(), key)?;
    server_crypto.alpn_protocols = vec![b"hyperlink".to_vec()];

    let quic_server_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
        .map_err(|e| anyhow::anyhow!("failed to create QuicServerConfig: {}", e))?;
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server_crypto));
    server_config.migration(true);

    // Set transport options.
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(Duration::from_secs(10).try_into()?));
    transport.keep_alive_interval(Some(Duration::from_secs(3)));
    transport.datagram_receive_buffer_size(Some(65536 * 8));
    transport.datagram_send_buffer_size(65536 * 8);
    server_config.transport_config(Arc::new(transport));

    let endpoint = Endpoint::server(server_config, bind_addr)?;
    info!("QUIC server listening on {}", endpoint.local_addr()?);

    let config_arc = Arc::new(Mutex::new(config));
    let config_path_arc = Arc::new(config_path);

    // Bounds how long an operator-initiated `--pair` window stays open (see
    // docs/SECURITY_REVIEW.md finding #3). The verifier only accepts unknown
    // certificates while `pairing_open` is set, so closing it here (after a
    // timeout, or once one pairing attempt resolves below) ends the
    // accept-any-cert window at the TLS layer. The GUI drives the same switch
    // through `set_pairing_open` and runs its own visible countdown.
    const PAIRING_WINDOW_TIMEOUT: Duration = Duration::from_secs(5 * 60);
    if is_pairing {
        let pairing_open_timeout = pairing_open.clone();
        tokio::spawn(async move {
            tokio::time::sleep(PAIRING_WINDOW_TIMEOUT).await;
            if pairing_open_timeout.swap(false, Ordering::SeqCst) {
                warn!(
                    "pairing window closed after {}s with no completed pairing",
                    PAIRING_WINDOW_TIMEOUT.as_secs()
                );
            }
        });
    }

    loop {
        let incoming = match endpoint.accept().await {
            Some(conn) => conn,
            None => break,
        };

        // Call accept to obtain Connecting
        let connecting = match incoming.accept() {
            Ok(c) => c,
            Err(e) => {
                error!("failed to accept incoming connection: {}", e);
                continue;
            }
        };

        let config_clone = config_arc.clone();
        let config_path_clone = config_path_arc.clone();
        let certs_clone = certs.clone();
        let trusted_clone = trusted_set.clone();

        tokio::spawn(async move {
            info!("incoming connection from client...");
            match handle_incoming_connection(
                connecting,
                certs_clone,
                trusted_clone,
                config_clone,
                config_path_clone,
            )
            .await
            {
                Ok(_) => {
                    info!("client session ended normally");
                }
                Err(e) => {
                    error!("client session error: {}", e);
                }
            }
        });
    }

    Ok(())
}

/// The user's answer to a pairing request.
#[derive(PartialEq, Eq)]
enum PairingAnswer {
    Accepted,
    Rejected,
    /// Nobody answered within `PAIRING_DECISION_TIMEOUT`.
    TimedOut,
}

/// Asks the user whether to trust a new phone. With the GUI, the request goes to
/// the main window; headless builds fall back to a terminal prompt.
async fn ask_user_to_pair(pin: u32, fingerprint: &str) -> PairingAnswer {
    #[cfg(feature = "video")]
    if let Some(tx) = crate::UI_SENDER.get() {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let request = crate::VideoGuiMessage::PairingRequest {
            pin,
            reply: reply_tx,
        };
        if tx.send(request).await.is_err() {
            return PairingAnswer::Rejected;
        }
        return match tokio::time::timeout(PAIRING_DECISION_TIMEOUT, reply_rx).await {
            Ok(Ok(true)) => PairingAnswer::Accepted,
            Ok(_) => PairingAnswer::Rejected,
            Err(_) => PairingAnswer::TimedOut,
        };
    }

    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║              PAIRING REQUEST RECEIVED                    ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!("  Client certificate fingerprint: {}", fingerprint);
    println!("  Mutual validation PIN:          {:06}", pin);
    println!("  Do you trust this device? (y/n): ");

    let read = tokio::task::spawn_blocking(|| {
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let trimmed = input.trim().to_lowercase();
            trimmed == "y" || trimmed == "yes"
        } else {
            false
        }
    });
    match tokio::time::timeout(PAIRING_DECISION_TIMEOUT, read).await {
        Ok(Ok(true)) => PairingAnswer::Accepted,
        Ok(_) => PairingAnswer::Rejected,
        Err(_) => PairingAnswer::TimedOut,
    }
}

async fn handle_incoming_connection(
    connecting: quinn::Connecting,
    our_certs: Vec<CertificateDer<'static>>,
    trusted_set: Arc<Mutex<HashSet<[u8; 32]>>>,
    config_arc: Arc<Mutex<DeviceConfig>>,
    config_path: Arc<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let connection = connecting.await?;
    info!("QUIC handshake complete!");

    // Read the fingerprint from this connection's own peer certificate rather than
    // the verifier's shared pending slot, which two simultaneous handshakes could
    // overwrite.
    let fp = connection
        .peer_identity()
        .and_then(|id| id.downcast::<Vec<CertificateDer<'static>>>().ok())
        .and_then(|certs| certs.first().map(|c| crypto::compute_fingerprint(c)))
        .ok_or_else(|| anyhow::anyhow!("client presented no certificate"))?;
    let fp_str = crypto::fingerprint_to_string(&fp);

    let already_trusted = trusted_set.lock().unwrap().contains(&fp);
    if !already_trusted {
        // The verifier only lets an unknown certificate through while pairing is
        // open, so this is a pairing attempt. Whatever the outcome, it uses up the
        // window: one attempt per window, as before.
        set_pairing_open(false);

        let our_fp = crypto::compute_fingerprint(&our_certs[0]);
        let pin = crypto::generate_pairing_pin(&fp, &our_fp);

        match ask_user_to_pair(pin, &fp_str).await {
            PairingAnswer::Accepted => {}
            PairingAnswer::Rejected => {
                warn!("pairing rejected, closing connection");
                connection.close(0u32.into(), b"pairing rejected");
                return Err(anyhow::anyhow!("pairing rejected by user"));
            }
            PairingAnswer::TimedOut => {
                warn!("pairing request went unanswered, closing connection");
                connection.close(0u32.into(), b"pairing timed out");
                return Err(anyhow::anyhow!("pairing request timed out"));
            }
        }

        // Every companion currently presents the same name, so a second phone gets
        // a fingerprint-suffixed entry rather than evicting the first phone's trust.
        {
            let mut config = config_arc.lock().unwrap();
            let key = config.add_trusted_peer_unique("Android-Companion", &fp_str);
            info!(
                "pairing accepted, trusting {:?} with fingerprint: {}",
                key, fp_str
            );
            config.save(&config_path)?;
            crate::refresh_proximity_trust(&config.trusted_peers);
        }
        trusted_set.lock().unwrap().insert(fp);
    } else {
        info!("paired client connected securely");
    }

    if let Some(server) = SERVER.get() {
        server
            .sessions
            .lock()
            .unwrap()
            .push((fp, connection.clone()));
    }
    let _session_guard = SessionGuard { fingerprint: fp };
    let device_name = config_arc
        .lock()
        .unwrap()
        .trusted_peers
        .iter()
        .find(|(_, v)| **v == fp_str)
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| "Android phone".to_string());
    notify_gui(crate::SessionEvent::PhoneConnected { device_name });

    // Set when the phone (re)starts a stream, so frame reassembly starts fresh.
    let video_reset = Arc::new(AtomicBool::new(false));
    let video_reset_uni = video_reset.clone();

    // Spawn background task to accept unidirectional streams (for VideoConfig)
    let conn_uni = connection.clone();
    tokio::spawn(async move {
        while let Ok(mut recv_stream) = conn_uni.accept_uni().await {
            let video_reset_uni = video_reset_uni.clone();
            tokio::spawn(async move {
                let mut stream_type_buf = [0u8; 1];
                if recv_stream.read_exact(&mut stream_type_buf).await.is_ok() {
                    let stream_type = stream_type_buf[0];
                    if stream_type == 0x31 {
                        if let Ok(buf) = recv_stream.read_to_end(65536).await {
                            if let Ok(header) = hyperlink_protocol::version::Header::decode(&buf) {
                                if header.message_type
                                    == hyperlink_protocol::message::MessageType::VideoConfig
                                {
                                    let payload = &buf[hyperlink_protocol::version::HEADER_SIZE..];
                                    if let Ok(config) =
                                        hyperlink_protocol::video::VideoConfig::decode(payload)
                                    {
                                        video_reset_uni.store(true, Ordering::SeqCst);
                                        info!(
                                            "received video config: SPS={} bytes, PPS={} bytes",
                                            config.sps.len(),
                                            config.pps.len()
                                        );
                                        #[cfg(feature = "video")]
                                        if let Some(sender) = crate::UI_SENDER.get() {
                                            let _ = sender
                                                .send(crate::VideoGuiMessage::Config {
                                                    sps: config.sps,
                                                    pps: config.pps,
                                                })
                                                .await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            });
        }
    });

    // Spawn background task to read datagrams (for VideoFrame)
    let conn_dg = connection.clone();
    let video_reset_dg = video_reset.clone();
    tokio::spawn(async move {
        let mut assembler = crate::video_assembler::FrameAssembler::new();
        let mut last_report = std::time::Instant::now();
        let mut last_keyframe_request: Option<std::time::Instant> = None;

        while let Ok(datagram) = conn_dg.read_datagram().await {
            let Ok(hl_header) = hyperlink_protocol::version::Header::decode(&datagram) else {
                continue;
            };
            if hl_header.message_type != hyperlink_protocol::message::MessageType::VideoFrame {
                continue;
            }
            let payload = &datagram[hyperlink_protocol::version::HEADER_SIZE..];
            let Ok(video_header) = hyperlink_protocol::video::VideoFrameHeader::decode(payload)
            else {
                continue;
            };
            // A new stream restarts the phone's frame numbering.
            if video_reset_dg.swap(false, Ordering::SeqCst) {
                assembler.reset();
            }

            let fragment = &payload[hyperlink_protocol::video::VIDEO_FRAME_HEADER_SIZE..];
            let out = assembler.on_fragment(&video_header, fragment);

            // Ask for a fresh keyframe after a loss, at most a few times a second:
            // one is enough to recover, and it takes a round trip to arrive.
            if out.request_keyframe
                && last_keyframe_request.is_none_or(|t| t.elapsed() >= KEYFRAME_REQUEST_INTERVAL)
            {
                last_keyframe_request = Some(std::time::Instant::now());
                #[cfg(feature = "video")]
                if let Some(tx) = crate::INPUT_SENDER.get() {
                    let _ = tx.try_send(crate::InputGuiMessage::KeyframeRequest);
                }
            }

            // Straight into the decoder from here: video never waits behind the
            // UI thread. The GUI only gets lightweight stats.
            #[cfg(feature = "video")]
            if let Some(frame) = &out.frame {
                if let Some(input) = crate::video_input() {
                    input.push_frame(&frame.data, frame.timestamp_us, frame.is_keyframe);
                }
                if let Some(sender) = crate::UI_SENDER.get() {
                    let _ = sender.try_send(crate::VideoGuiMessage::Frame {
                        bytes: frame.data.len(),
                        width: frame.width,
                        height: frame.height,
                    });
                }
            }
            #[cfg(not(feature = "video"))]
            let _ = out.frame;

            if last_report.elapsed() >= Duration::from_secs(5) {
                if assembler.frames_lost > 0 {
                    warn!(
                        frames_ok = assembler.frames_ok,
                        frames_lost = assembler.frames_lost,
                        "video frames lost in the last 5s"
                    );
                } else {
                    debug!(
                        frames_ok = assembler.frames_ok,
                        "video frames received in the last 5s"
                    );
                }
                assembler.frames_ok = 0;
                assembler.frames_lost = 0;
                last_report = std::time::Instant::now();
            }
        }
    });

    // Phase 3: Open bidirectional input stream (type 0x40) from host to companion
    #[cfg(feature = "video")]
    let conn_clone = connection.clone();
    #[cfg(feature = "video")]
    tokio::spawn(async move {
        match conn_clone.open_bi().await {
            Ok((mut send_stream, mut recv_stream)) => {
                info!("opened bidirectional input stream 0x40");
                if let Err(e) = send_stream.write_all(&[0x40]).await {
                    error!("failed to write input stream header: {}", e);
                    return;
                }

                // Spawn reader for input ACKs (round-trip validation)
                tokio::spawn(async move {
                    let mut ack_buf = [0u8; 64];
                    while let Ok(Some(n)) = recv_stream.read(&mut ack_buf).await {
                        if n >= hyperlink_protocol::version::HEADER_SIZE
                            + hyperlink_protocol::input::INPUT_ACK_SIZE
                        {
                            if let Ok(ack) = hyperlink_protocol::input::InputAck::decode(
                                &ack_buf[hyperlink_protocol::version::HEADER_SIZE
                                    ..hyperlink_protocol::version::HEADER_SIZE
                                        + hyperlink_protocol::input::INPUT_ACK_SIZE],
                            ) {
                                debug!(
                                    seq = ack.seq,
                                    timestamp_us = ack.timestamp_us,
                                    "received input ack from companion"
                                );
                            }
                        }
                    }
                });

                // Read from INPUT_RECEIVER and write serialized packets to send_stream
                if let Some(receiver) = crate::INPUT_RECEIVER.get() {
                    let rx = receiver.clone();
                    let mut packet = Vec::with_capacity(64);
                    let mut payload = Vec::with_capacity(32);
                    while let Ok(msg) = rx.recv().await {
                        payload.clear();
                        packet.clear();

                        let (msg_type, enc_res) = match msg {
                            crate::InputGuiMessage::Pointer(p) => (
                                hyperlink_protocol::message::MessageType::PointerEvent,
                                p.encode(&mut payload),
                            ),
                            crate::InputGuiMessage::Key(k) => (
                                hyperlink_protocol::message::MessageType::KeyEvent,
                                k.encode(&mut payload),
                            ),
                            crate::InputGuiMessage::Scroll(s) => (
                                hyperlink_protocol::message::MessageType::ScrollEvent,
                                s.encode(&mut payload),
                            ),
                            crate::InputGuiMessage::Nav(n) => (
                                hyperlink_protocol::message::MessageType::NavEvent,
                                n.encode(&mut payload),
                            ),
                            crate::InputGuiMessage::Dnd(d) => (
                                hyperlink_protocol::message::MessageType::DndSync,
                                d.encode(&mut payload),
                            ),
                            crate::InputGuiMessage::NotificationAction(a) => (
                                hyperlink_protocol::message::MessageType::NotificationActionInvoke,
                                a.encode(&mut payload),
                            ),
                            crate::InputGuiMessage::NotificationDismiss(dm) => (
                                hyperlink_protocol::message::MessageType::NotificationDismiss,
                                dm.encode(&mut payload),
                            ),
                            crate::InputGuiMessage::KeyframeRequest => (
                                hyperlink_protocol::message::MessageType::KeyframeRequest,
                                Ok(()),
                            ),
                        };

                        if enc_res.is_ok() {
                            let header = hyperlink_protocol::version::Header::new(
                                msg_type,
                                payload.len() as u32,
                            );
                            if header.encode(&mut packet).is_ok() {
                                packet.extend_from_slice(&payload);
                                if send_stream.write_all(&packet).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                debug!("could not open bidirectional input stream: {}", e);
            }
        }
    });

    // Phase 5: Initialize clipboard manager and open dedicated stream 0x70 to companion
    let clipboard_mgr = crate::clipboard::ClipboardManager::new();
    let conn_for_clip = connection.clone();
    let clip_mgr_for_watch = clipboard_mgr.clone();
    tokio::spawn(async move {
        match conn_for_clip.open_bi().await {
            Ok((mut send_stream, mut recv_stream)) => {
                if send_stream.write_all(&[0x70]).await.is_ok() {
                    info!("opened dedicated bidirectional clipboard stream 0x70 to companion");
                    let (clip_tx, mut clip_rx) = tokio::sync::mpsc::unbounded_channel();
                    let _watcher = clip_mgr_for_watch.start_watcher(clip_tx);

                    // Spawn reader for ClipboardAck
                    tokio::spawn(async move {
                        let mut hdr_bytes = [0u8; hyperlink_protocol::version::HEADER_SIZE];
                        while recv_stream.read_exact(&mut hdr_bytes).await.is_ok() {
                            if let Ok(hdr) = hyperlink_protocol::version::Header::decode(&hdr_bytes)
                            {
                                let mut ack_buf = vec![0u8; hdr.payload_len as usize];
                                if recv_stream.read_exact(&mut ack_buf).await.is_err() {
                                    break;
                                }
                                if hdr.message_type
                                    == hyperlink_protocol::message::MessageType::ClipboardAck
                                {
                                    if let Ok(ack) =
                                        hyperlink_protocol::clipboard::ClipboardAck::decode(
                                            &ack_buf,
                                        )
                                    {
                                        debug!(seq = ack.seq, from = %ack.origin_id, "received ClipboardAck");
                                    }
                                }
                            }
                        }
                    });

                    // Forward local clipboard updates to companion
                    while let Some(msg) = clip_rx.recv().await {
                        let mut payload = Vec::new();
                        if msg.encode(&mut payload).is_ok() {
                            let hdr = hyperlink_protocol::version::Header::new(
                                hyperlink_protocol::message::MessageType::ClipboardMessage,
                                payload.len() as u32,
                            );
                            let mut packet = Vec::with_capacity(
                                hyperlink_protocol::version::HEADER_SIZE + payload.len(),
                            );
                            if hdr.encode(&mut packet).is_ok() {
                                packet.extend_from_slice(&payload);
                                if send_stream.write_all(&packet).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                debug!("could not open dedicated clipboard stream 0x70: {}", e);
            }
        }
    });

    // Phase 6: Open dedicated VFS stream (type 0x80) from host to companion
    let conn_vfs = connection.clone();
    let vfs_client = std::sync::Arc::new(crate::vfs::VfsClient::new());
    let vfs_cache = std::sync::Arc::new(crate::vfs::FileChunkCache::default());
    let multipath_mgr = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::multipath::HostMultipathManager::new(),
    ));
    multipath_mgr.lock().await.register_path(
        hyperlink_protocol::resilience::PATH_ID_PRIMARY_WIFI,
        connection.remote_address(),
    );

    // Phase 8: Host proximity, pre-warming, and workflow state manager
    let workflow_path = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("workflow_state.json");
    let trusted_peers = { config_arc.lock().unwrap().trusted_peers.clone() };
    let proximity_mgr = std::sync::Arc::new(crate::proximity::HostProximityManager::new(
        workflow_path,
        trusted_peers,
    ));

    // Phase 9: Host scoped app-state handoff manager
    let drafts_dir = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("notes");
    let handoff_mgr = std::sync::Arc::new(crate::handoff::HostHandoffManager::new(drafts_dir));

    // Phase 10: Host ambient context event bus and agent
    let journal_path = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("ambient_events.jsonl");
    let ambient_bus = std::sync::Arc::new(crate::ambient::AmbientEventBus::new(Some(journal_path)));
    let ambient_agent = std::sync::Arc::new(std::sync::Mutex::new(
        crate::ambient::AmbientContextAgent::new(
            (*ambient_bus).clone(),
            hyperlink_protocol::ambient::AgentConsentPolicy::default(),
        ),
    ));

    // Phase 7: Host link health and auto-failover evaluator
    let mp_monitor = multipath_mgr.clone();
    let conn_monitor = connection.clone();
    tokio::spawn(async move {
        let mut check_interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            check_interval.tick().await;
            let rtt = conn_monitor.rtt();
            let rtt_us = rtt.as_micros() as u32;

            let mut mp = mp_monitor.lock().await;
            let active_id = mp.active_path_id;
            if let Some(active) = mp.paths.get_mut(&active_id) {
                active.rtt_us = rtt_us;
                active.last_seen_us = hyperlink_protocol::clock::now_us();
            }

            if let Some((target_path, reason)) = mp.should_failover() {
                if let Some(notice) = mp.initiate_failover(target_path, reason) {
                    info!(
                        from = notice.from_path,
                        to = notice.to_path,
                        reason = ?reason,
                        "host multipath monitor triggered automatic failover"
                    );
                    let mut payload = Vec::new();
                    if notice.encode(&mut payload).is_ok() {
                        let header = hyperlink_protocol::version::Header::new(
                            hyperlink_protocol::message::MessageType::PathSwitchNotice,
                            payload.len() as u32,
                        );
                        let mut packet = Vec::with_capacity(10 + payload.len());
                        if header.encode(&mut packet).is_ok() {
                            packet.extend_from_slice(&payload);
                            if let Ok((mut send_stream, _)) = conn_monitor.open_bi().await {
                                let _ = send_stream.write_all(&[0x50]).await;
                                let _ = send_stream.write_all(&packet).await;
                                let _ = send_stream.finish();
                            }
                        }
                    }
                }
            }
        }
    });

    tokio::spawn(async move {
        match conn_vfs.open_bi().await {
            Ok((mut send_stream, recv_stream)) => {
                info!("opened dedicated VFS stream 0x80 to companion");
                if let Err(e) = send_stream.write_all(&[0x80]).await {
                    error!("failed to write VFS stream header: {}", e);
                    return;
                }
                vfs_client.set_stream(send_stream, recv_stream).await;

                // Determine default mount point
                let default_mnt = std::env::var("XDG_RUNTIME_DIR")
                    .map(|d| std::path::PathBuf::from(d).join("hyperlink"))
                    .unwrap_or_else(|_| {
                        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                        std::path::PathBuf::from(home).join("HyperLink")
                    });

                let mnt_str = default_mnt.to_string_lossy().to_string();
                let client_clone = vfs_client.clone();
                let cache_clone = vfs_cache.clone();
                let rt_handle = tokio::runtime::Handle::current();
                let conn_for_mount = conn_vfs.clone();

                // Keep the mount for exactly as long as the phone is connected:
                // dropping the session unmounts it. This is a plain thread (not a
                // runtime worker), so blocking on the connection here is fine.
                std::thread::spawn(move || {
                    match crate::vfs::mount_fuse(
                        &mnt_str,
                        client_clone,
                        cache_clone,
                        rt_handle.clone(),
                    ) {
                        Ok(session) => {
                            rt_handle.block_on(conn_for_mount.closed());
                            drop(session);
                            info!(mountpoint = %mnt_str, "phone disconnected; unmounted its files");
                        }
                        Err(e) => {
                            warn!(error = %e, "FUSE mount not started (non-fatal, e.g. unprivileged sandbox or missing mountpoint)");
                        }
                    }
                });
            }
            Err(e) => {
                debug!("could not open dedicated VFS stream 0x80: {}", e);
            }
        }
    });

    // Accept bidirectional streams (the client opens the control plane stream).
    loop {
        match connection.accept_bi().await {
            Ok((mut send_stream, mut recv_stream)) => {
                // Read the first stream category byte (multiplex scaffold).
                let mut stream_type_buf = [0u8; 1];
                if recv_stream.read_exact(&mut stream_type_buf).await.is_err() {
                    break;
                }

                let stream_type = stream_type_buf[0];
                info!(
                    "new bidirectional stream accepted, type: 0x{:02X}",
                    stream_type
                );

                if stream_type == 0x50 {
                    // Control plane stream. Handles notification sync, DND, clipboard, multipath, proximity, handoff, and ambient.
                    let clip_mgr_inbound = clipboard_mgr.clone();
                    let mp_mgr_control = multipath_mgr.clone();
                    let prox_mgr_control = proximity_mgr.clone();
                    let handoff_mgr_control = handoff_mgr.clone();
                    let ambient_bus_control = ambient_bus.clone();
                    let ambient_agent_control = ambient_agent.clone();
                    tokio::spawn(async move {
                        let mut hdr_bytes = [0u8; hyperlink_protocol::version::HEADER_SIZE];
                        loop {
                            if recv_stream.read_exact(&mut hdr_bytes).await.is_err() {
                                info!("control stream closed by client");
                                break;
                            }

                            if let Ok(header) =
                                hyperlink_protocol::version::Header::decode(&hdr_bytes)
                            {
                                let payload_len = header.payload_len as usize;
                                let mut payload = vec![0u8; payload_len];
                                if recv_stream.read_exact(&mut payload).await.is_err() {
                                    break;
                                }

                                match header.message_type {
                                    hyperlink_protocol::message::MessageType::NotificationPost => {
                                        let host_received_us =
                                            hyperlink_protocol::clock::now_us();
                                        if let Ok(notif) =
                                            hyperlink_protocol::notification::NotificationPost::decode(
                                                &payload,
                                            )
                                        {
                                            info!(
                                                id = %notif.id,
                                                app = %notif.app_name,
                                                title = %notif.title,
                                                "received notification from device"
                                            );

                                            // Send NotificationAck back to client
                                            let ack =
                                                hyperlink_protocol::notification::NotificationAck {
                                                    id: notif.id.clone(),
                                                    device_timestamp_ms: notif.timestamp_ms,
                                                    host_received_us,
                                                };
                                            let mut ack_payload = Vec::new();
                                            if ack.encode(&mut ack_payload).is_ok() {
                                                let ack_hdr =
                                                    hyperlink_protocol::version::Header::new(
                                                        hyperlink_protocol::message::MessageType::NotificationAck,
                                                        ack_payload.len() as u32,
                                                    );
                                                let mut ack_packet = Vec::with_capacity(
                                                    hyperlink_protocol::version::HEADER_SIZE
                                                        + ack_payload.len(),
                                                );
                                                if ack_hdr.encode(&mut ack_packet).is_ok() {
                                                    ack_packet.extend_from_slice(&ack_payload);
                                                    let _ = send_stream.write_all(&ack_packet).await;
                                                }
                                            }

                                            #[cfg(not(feature = "video"))]
                                            {
                                                if !crate::is_dnd_active() {
                                                    crate::dispatch_desktop_notification(&notif);
                                                }
                                            }

                                            #[cfg(feature = "video")]
                                            if let Some(tx) = crate::UI_SENDER.get() {
                                                let _ = tx.try_send(
                                                    crate::VideoGuiMessage::Notification(notif.clone()),
                                                );
                                            }

                                            // Auto-publish ambient notification event
                                            let amb_evt =
                                                hyperlink_protocol::ambient::AmbientEvent::new(
                                                    hyperlink_protocol::clock::now_us(),
                                                    hyperlink_protocol::ambient::AmbientEventCategory::Notifications,
                                                    "phone:AndroidCompanion",
                                                    host_received_us,
                                                    format!("{}: {}", notif.app_name, notif.title),
                                                    format!(
                                                        r#"{{"id":"{}","app":"{}","title":"{}"}}"#,
                                                        notif.id, notif.package_name, notif.title
                                                    ),
                                                );
                                            if let Ok(evt) = amb_evt {
                                                let _ = ambient_bus_control.publish(evt);
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::NotificationDismiss => {
                                        if let Ok(dismiss) =
                                            hyperlink_protocol::notification::NotificationDismiss::decode(
                                                &payload,
                                            )
                                        {
                                            debug!(id = %dismiss.id, "device dismissed notification");
                                            #[cfg(feature = "video")]
                                            if let Some(tx) = crate::UI_SENDER.get() {
                                                let _ = tx.try_send(
                                                    crate::VideoGuiMessage::NotificationDismiss(
                                                        dismiss.id,
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::DndSync => {
                                        if let Ok(dnd) =
                                            hyperlink_protocol::notification::DndSync::decode(
                                                &payload,
                                            )
                                        {
                                            info!(enabled = dnd.dnd_enabled, "device updated DND status");
                                            crate::set_global_dnd_active(dnd.dnd_enabled);
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::ClipboardMessage => {
                                        if let Ok(clip) =
                                            hyperlink_protocol::clipboard::ClipboardMessage::decode(&payload)
                                        {
                                            let ack =
                                                hyperlink_protocol::clipboard::ClipboardAck {
                                                    seq: clip.seq,
                                                    content_hash: clip.content_hash,
                                                    origin_id: clip.origin_id.clone(),
                                                    host_received_us:
                                                        hyperlink_protocol::clock::now_us(),
                                                };
                                            let mut ack_payload = Vec::new();
                                            if ack.encode(&mut ack_payload).is_ok() {
                                                let ack_hdr =
                                                    hyperlink_protocol::version::Header::new(
                                                        hyperlink_protocol::message::MessageType::ClipboardAck,
                                                        ack_payload.len() as u32,
                                                    );
                                                let mut ack_packet = Vec::with_capacity(
                                                    hyperlink_protocol::version::HEADER_SIZE
                                                        + ack_payload.len(),
                                                );
                                                if ack_hdr.encode(&mut ack_packet).is_ok() {
                                                    ack_packet.extend_from_slice(&ack_payload);
                                                    let _ = send_stream.write_all(&ack_packet).await;
                                                }
                                            }
                                            if let Err(e) = clip_mgr_inbound.apply_remote_clip(&clip) {
                                                warn!("failed to apply remote clip: {}", e);
                                            }

                                            // Auto-publish ambient clipboard event
                                            let amb_clip =
                                                hyperlink_protocol::ambient::AmbientEvent::new(
                                                    hyperlink_protocol::clock::now_us(),
                                                    hyperlink_protocol::ambient::AmbientEventCategory::Clipboard,
                                                    "phone:AndroidCompanion",
                                                    hyperlink_protocol::clock::now_us(),
                                                    format!(
                                                        "Clipboard sync ({} bytes, mime: {})",
                                                        clip.payload.len(),
                                                        clip.mime_type
                                                    ),
                                                    format!(
                                                        r#"{{"mime":"{}","len":{}}}"#,
                                                        clip.mime_type,
                                                        clip.payload.len()
                                                    ),
                                                );
                                            if let Ok(evt) = amb_clip {
                                                let _ = ambient_bus_control.publish(evt);
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::PathProbeRequest => {
                                        if let Ok(probe_req) =
                                            hyperlink_protocol::resilience::PathProbeRequest::decode(&payload)
                                        {
                                            let probe_resp = mp_mgr_control.lock().await.handle_probe_request(&probe_req);
                                            let mut resp_payload = Vec::new();
                                            if probe_resp.encode(&mut resp_payload).is_ok() {
                                                let resp_hdr = hyperlink_protocol::version::Header::new(
                                                    hyperlink_protocol::message::MessageType::PathProbeResponse,
                                                    resp_payload.len() as u32,
                                                );
                                                let mut pkt = Vec::with_capacity(10 + resp_payload.len());
                                                if resp_hdr.encode(&mut pkt).is_ok() {
                                                    pkt.extend_from_slice(&resp_payload);
                                                    let _ = send_stream.write_all(&pkt).await;
                                                }
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::PathProbeResponse => {
                                        if let Ok(probe_resp) =
                                            hyperlink_protocol::resilience::PathProbeResponse::decode(&payload)
                                        {
                                            mp_mgr_control.lock().await.handle_probe_response(&probe_resp);
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::PathSwitchNotice => {
                                        if let Ok(notice) =
                                            hyperlink_protocol::resilience::PathSwitchNotice::decode(&payload)
                                        {
                                            let ack = mp_mgr_control.lock().await.handle_switch_notice(&notice);
                                            let mut ack_payload = Vec::new();
                                            if ack.encode(&mut ack_payload).is_ok() {
                                                let ack_hdr = hyperlink_protocol::version::Header::new(
                                                    hyperlink_protocol::message::MessageType::PathSwitchAck,
                                                    ack_payload.len() as u32,
                                                );
                                                let mut pkt = Vec::with_capacity(10 + ack_payload.len());
                                                if ack_hdr.encode(&mut pkt).is_ok() {
                                                    pkt.extend_from_slice(&ack_payload);
                                                    let _ = send_stream.write_all(&pkt).await;
                                                }
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::PathSwitchAck => {
                                        if let Ok(ack) =
                                            hyperlink_protocol::resilience::PathSwitchAck::decode(&payload)
                                        {
                                            mp_mgr_control.lock().await.handle_switch_ack(&ack);
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::PathHealthReport => {
                                        if let Ok(report) =
                                            hyperlink_protocol::resilience::PathHealthReport::decode(&payload)
                                        {
                                            mp_mgr_control.lock().await.handle_health_report(&report);
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::ProximityBeacon => {
                                        if let Ok(beacon) =
                                            hyperlink_protocol::proximity::ProximityBeacon::decode(&payload)
                                        {
                                            let ack = prox_mgr_control.handle_proximity_beacon(&beacon);
                                            let mut ack_payload = Vec::new();
                                            if ack.encode(&mut ack_payload).is_ok() {
                                                let ack_hdr = hyperlink_protocol::version::Header::new(
                                                    hyperlink_protocol::message::MessageType::ProximityAck,
                                                    ack_payload.len() as u32,
                                                );
                                                let mut pkt = Vec::with_capacity(10 + ack_payload.len());
                                                if ack_hdr.encode(&mut pkt).is_ok() {
                                                    pkt.extend_from_slice(&ack_payload);
                                                    let _ = send_stream.write_all(&pkt).await;
                                                }
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::PreWarmState => {
                                        if let Ok(state) =
                                            hyperlink_protocol::proximity::PreWarmState::decode(&payload)
                                        {
                                            debug!(warmed = state.warmed, power = state.power_profile, "received peer pre-warm state");
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::WorkflowStateSave => {
                                        if let Ok(wf) =
                                            hyperlink_protocol::proximity::WorkflowState::decode(&payload)
                                        {
                                            let ack = prox_mgr_control.save_workflow_state(wf).unwrap_or(
                                                hyperlink_protocol::proximity::WorkflowStateAck {
                                                    restored: false,
                                                    status_code: 1,
                                                    timestamp_us: hyperlink_protocol::clock::now_us(),
                                                }
                                            );
                                            let mut ack_payload = Vec::new();
                                            if ack.encode(&mut ack_payload).is_ok() {
                                                let ack_hdr = hyperlink_protocol::version::Header::new(
                                                    hyperlink_protocol::message::MessageType::WorkflowStateAck,
                                                    ack_payload.len() as u32,
                                                );
                                                let mut pkt = Vec::with_capacity(10 + ack_payload.len());
                                                if ack_hdr.encode(&mut pkt).is_ok() {
                                                    pkt.extend_from_slice(&ack_payload);
                                                    let _ = send_stream.write_all(&pkt).await;
                                                }
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::WorkflowStateRestore => {
                                        let saved_wf = prox_mgr_control.load_saved_workflow_state();
                                        let mut resp_payload = Vec::new();
                                        if saved_wf.encode(&mut resp_payload).is_ok() {
                                            let resp_hdr = hyperlink_protocol::version::Header::new(
                                                hyperlink_protocol::message::MessageType::WorkflowStateSave,
                                                resp_payload.len() as u32,
                                            );
                                            let mut pkt = Vec::with_capacity(10 + resp_payload.len());
                                            if resp_hdr.encode(&mut pkt).is_ok() {
                                                pkt.extend_from_slice(&resp_payload);
                                                let _ = send_stream.write_all(&pkt).await;
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::HandoffPayload => {
                                        if let Ok(handoff) =
                                            hyperlink_protocol::handoff::HandoffPayload::decode(&payload)
                                        {
                                            let ack = handoff_mgr_control.handle_incoming_handoff(&handoff);
                                            let mut ack_payload = Vec::new();
                                            if ack.encode(&mut ack_payload).is_ok() {
                                                let ack_hdr = hyperlink_protocol::version::Header::new(
                                                    hyperlink_protocol::message::MessageType::HandoffAck,
                                                    ack_payload.len() as u32,
                                                );
                                                let mut pkt = Vec::with_capacity(10 + ack_payload.len());
                                                if ack_hdr.encode(&mut pkt).is_ok() {
                                                    pkt.extend_from_slice(&ack_payload);
                                                    let _ = send_stream.write_all(&pkt).await;
                                                }
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::HandoffAck => {
                                        if let Ok(ack) =
                                            hyperlink_protocol::handoff::HandoffAck::decode(&payload)
                                        {
                                            debug!(
                                                session_id = ack.session_id,
                                                accepted = ack.accepted,
                                                "received HandoffAck from peer"
                                            );
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::HandoffDiscover => {
                                        let caps = handoff_mgr_control.get_capabilities();
                                        if let Ok(cap_json) = serde_json::to_string(&caps) {
                                            let cap_bytes = cap_json.as_bytes();
                                            let resp_hdr = hyperlink_protocol::version::Header::new(
                                                hyperlink_protocol::message::MessageType::HandoffDiscoverAck,
                                                cap_bytes.len() as u32,
                                            );
                                            let mut pkt = Vec::with_capacity(10 + cap_bytes.len());
                                            if resp_hdr.encode(&mut pkt).is_ok() {
                                                pkt.extend_from_slice(cap_bytes);
                                                let _ = send_stream.write_all(&pkt).await;
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::HandoffDiscoverAck => {
                                        debug!("received peer handoff capability manifest");
                                    }
                                    hyperlink_protocol::message::MessageType::AmbientEventPublish => {
                                        if let Ok(event) =
                                            hyperlink_protocol::ambient::AmbientEvent::decode(&payload)
                                        {
                                            let event_id = event.event_id;
                                            let _ = ambient_bus_control.publish(event);
                                            let ack_hdr = hyperlink_protocol::version::Header::new(
                                                hyperlink_protocol::message::MessageType::AmbientEventAck,
                                                8,
                                            );
                                            let mut pkt = Vec::with_capacity(18);
                                            if ack_hdr.encode(&mut pkt).is_ok() {
                                                pkt.extend_from_slice(&event_id.to_be_bytes());
                                                let _ = send_stream.write_all(&pkt).await;
                                            }
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::AmbientEventAck => {
                                        debug!("received AmbientEventAck from peer");
                                    }
                                    hyperlink_protocol::message::MessageType::AgentConsentUpdate => {
                                        if let Ok(policy) =
                                            hyperlink_protocol::ambient::AgentConsentPolicy::decode(&payload)
                                        {
                                            ambient_agent_control.lock().unwrap().set_consent_policy(policy);
                                            debug!("updated ambient agent consent policy");
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::AgentConsentQuery => {
                                        let policy = ambient_agent_control
                                            .lock()
                                            .unwrap()
                                            .consent_policy()
                                            .clone();
                                        let mut policy_bytes = Vec::new();
                                        policy.encode(&mut policy_bytes);
                                        let resp_hdr = hyperlink_protocol::version::Header::new(
                                            hyperlink_protocol::message::MessageType::AgentConsentUpdate,
                                            policy_bytes.len() as u32,
                                        );
                                        let mut pkt = Vec::with_capacity(10 + policy_bytes.len());
                                        if resp_hdr.encode(&mut pkt).is_ok() {
                                            pkt.extend_from_slice(&policy_bytes);
                                            let _ = send_stream.write_all(&pkt).await;
                                        }
                                    }
                                    hyperlink_protocol::message::MessageType::AgentQueryRequest => {
                                        if let Ok(req) =
                                            hyperlink_protocol::ambient::AgentQueryRequest::decode(&payload)
                                        {
                                            let resp = ambient_agent_control
                                                .lock()
                                                .unwrap()
                                                .process_query(&req);
                                            let mut resp_bytes = Vec::new();
                                            if resp.encode(&mut resp_bytes).is_ok() {
                                                let resp_hdr = hyperlink_protocol::version::Header::new(
                                                    hyperlink_protocol::message::MessageType::AgentQueryResponse,
                                                    resp_bytes.len() as u32,
                                                );
                                                let mut pkt = Vec::with_capacity(10 + resp_bytes.len());
                                                if resp_hdr.encode(&mut pkt).is_ok() {
                                                    pkt.extend_from_slice(&resp_bytes);
                                                    let _ = send_stream.write_all(&pkt).await;
                                                }
                                            }
                                        }
                                    }
                                    _ => {
                                        // Echo back unknown or heartbeat control packets
                                        let mut echo_packet = Vec::with_capacity(
                                            hyperlink_protocol::version::HEADER_SIZE + payload.len(),
                                        );
                                        if header.encode(&mut echo_packet).is_ok() {
                                            echo_packet.extend_from_slice(&payload);
                                            let _ = send_stream.write_all(&echo_packet).await;
                                        }
                                    }
                                }
                            } else {
                                // Non-framed legacy heartbeat echo
                                let mut echo = vec![0u8; 10];
                                echo.copy_from_slice(&hdr_bytes);
                                let _ = send_stream.write_all(&echo).await;
                            }
                        }
                    });
                } else if stream_type == 0x40 {
                    info!("client opened input stream 0x40");
                } else if stream_type == 0x70 {
                    info!("companion opened dedicated clipboard stream 0x70");
                    let clip_mgr_ded = clipboard_mgr.clone();
                    tokio::spawn(async move {
                        let mut hdr_bytes = [0u8; hyperlink_protocol::version::HEADER_SIZE];
                        loop {
                            if recv_stream.read_exact(&mut hdr_bytes).await.is_err() {
                                break;
                            }
                            if let Ok(header) =
                                hyperlink_protocol::version::Header::decode(&hdr_bytes)
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
                                        let host_received_us = hyperlink_protocol::clock::now_us();
                                        let ack = hyperlink_protocol::clipboard::ClipboardAck {
                                            seq: clip.seq,
                                            content_hash: clip.content_hash,
                                            origin_id: clip.origin_id.clone(),
                                            host_received_us,
                                        };
                                        let mut ack_payload = Vec::new();
                                        if ack.encode(&mut ack_payload).is_ok() {
                                            let ack_hdr = hyperlink_protocol::version::Header::new(
                                                hyperlink_protocol::message::MessageType::ClipboardAck,
                                                ack_payload.len() as u32,
                                            );
                                            let mut ack_packet = Vec::with_capacity(
                                                hyperlink_protocol::version::HEADER_SIZE
                                                    + ack_payload.len(),
                                            );
                                            if ack_hdr.encode(&mut ack_packet).is_ok() {
                                                ack_packet.extend_from_slice(&ack_payload);
                                                let _ = send_stream.write_all(&ack_packet).await;
                                            }
                                        }
                                        if let Err(e) = clip_mgr_ded.apply_remote_clip(&clip) {
                                            warn!("failed to apply remote clip: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                    });
                } else {
                    debug!("ignoring unhandled stream type: 0x{:02X}", stream_type);
                }
            }
            Err(e) => {
                info!("connection ended: {}", e);
                break;
            }
        }
    }

    Ok(())
}
