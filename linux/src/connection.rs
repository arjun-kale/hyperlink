//! Quinn server configuration, mutual TLS (mTLS) verifications, and client session loops.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quinn::Endpoint;
use rustls::pki_types::CertificateDer;
use tracing::{debug, error, info, warn};

use hyperlink_protocol::config::DeviceConfig;
use hyperlink_protocol::crypto::{
    self, PendingPairingState, TofuClientVerifier,
};

/// Starts the QUIC server and listens for incoming connections.
pub async fn start_server(
    bind_addr: SocketAddr,
    config: DeviceConfig,
    config_path: std::path::PathBuf,
    is_pairing: bool,
) -> anyhow::Result<()> {
    // Parse certificate and key.
    let certs = rustls_pemfile::certs(&mut config.cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut config.key_pem.as_bytes())?
        .ok_or_else(|| anyhow::anyhow!("private key missing in host config"))?;

    let trusted_set = Arc::new(Mutex::new(config.get_trusted_fingerprints_set()));
    let pending_state = Arc::new(Mutex::new(PendingPairingState::default()));

    // Custom client verifier.
    let verifier = Arc::new(TofuClientVerifier::new(
        trusted_set.clone(),
        is_pairing,
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
    
    // Set transport options.
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(Duration::from_secs(10).try_into()?));
    transport.keep_alive_interval(Some(Duration::from_secs(3)));
    server_config.transport_config(Arc::new(transport));

    let endpoint = Endpoint::server(server_config, bind_addr)?;
    info!("QUIC server listening on {}", endpoint.local_addr()?);

    let config_arc = Arc::new(Mutex::new(config));
    let config_path_arc = Arc::new(config_path);

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
        let pending_state_clone = pending_state.clone();
        let certs_clone = certs.clone();

        tokio::spawn(async move {
            info!("incoming connection from client...");
            match handle_incoming_connection(
                connecting,
                is_pairing,
                certs_clone,
                pending_state_clone,
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

async fn handle_incoming_connection(
    connecting: quinn::Connecting,
    is_pairing: bool,
    our_certs: Vec<CertificateDer<'static>>,
    pending_state: Arc<Mutex<PendingPairingState>>,
    config_arc: Arc<Mutex<DeviceConfig>>,
    config_path: Arc<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let connection = connecting.await?;
    info!("QUIC handshake complete!");

    if is_pairing {
        let peer_fp = {
            let state = pending_state.lock().unwrap();
            state.peer_fingerprint
        };
        if let Some(fp) = peer_fp {
            let our_fp = crypto::compute_fingerprint(&our_certs[0]);
            let pin = crypto::generate_pairing_pin(&fp, &our_fp);

            println!("\n╔══════════════════════════════════════════════════════════╗");
            println!("║              PAIRING REQUEST RECEIVED                    ║");
            println!("╚══════════════════════════════════════════════════════════╝");
            println!("  Client certificate fingerprint: {}", crypto::fingerprint_to_string(&fp));
            println!("  Mutual validation PIN:          {:06}", pin);
            println!("  Do you trust this device? (y/n): ");

            // Wait for user confirmation in terminal.
            let confirmed = tokio::task::spawn_blocking(move || {
                let mut input = String::new();
                if std::io::stdin().read_line(&mut input).is_ok() {
                    let trimmed = input.trim().to_lowercase();
                    trimmed == "y" || trimmed == "yes"
                } else {
                    false
                }
            })
            .await?;

            if !confirmed {
                warn!("pairing rejected by user, closing connection");
                connection.close(0u32.into(), b"pairing rejected");
                return Err(anyhow::anyhow!("pairing rejected by user"));
            }

            // Save the trusted client fingerprint.
            let fp_str = crypto::fingerprint_to_string(&fp);
            info!("pairing accepted, saving client fingerprint: {}", fp_str);

            {
                let mut config = config_arc.lock().unwrap();
                config.add_trusted_peer("Android-Companion", &fp_str);
                config.save(&config_path)?;
            }
        } else {
            connection.close(0u32.into(), b"pairing error");
            return Err(anyhow::anyhow!("failed to capture client certificate fingerprint"));
        }
    } else {
        info!("paired client connected securely");
    }

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
                info!("new bidirectional stream accepted, type: 0x{:02X}", stream_type);

                if stream_type == 0x50 {
                    // Control plane stream. Start heartbeat echo loop.
                    tokio::spawn(async move {
                        let mut buf = [0u8; 1500];
                        loop {
                            match recv_stream.read(&mut buf).await {
                                Ok(Some(n)) => {
                                    debug!("received {} bytes on control stream", n);
                                    // Echo back to client (heartbeat validation).
                                    if send_stream.write_all(&buf[..n]).await.is_err() {
                                        break;
                                    }
                                }
                                Ok(None) => {
                                    info!("control stream closed by client");
                                    break;
                                }
                                Err(e) => {
                                    error!("control stream read error: {}", e);
                                    break;
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
