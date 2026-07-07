//! Cryptographic helpers, Mutual TLS support, and TOFU certificate verifiers.
//!
//! Provides the core Trust-On-First-Use verifier implementations and
//! cert generation using `rcgen`.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::DistinguishedName;
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Compute the SHA-256 fingerprint of a DER-encoded certificate.
pub fn compute_fingerprint(cert_der: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(cert_der);
    let result = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&result);
    arr
}

/// Convert a fingerprint hash to a colon-separated hex string (e.g. "AA:BB:CC...").
pub fn fingerprint_to_string(fp: &[u8; 32]) -> String {
    fp.iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<String>>()
        .join(":")
}

/// Parse a colon-separated hex string back into a 32-byte fingerprint array.
pub fn string_to_fingerprint(s: &str) -> Result<[u8; 32], &'static str> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 32 {
        return Err("fingerprint must contain exactly 32 hex bytes separated by colons");
    }
    let mut fp = [0u8; 32];
    for (i, part) in parts.iter().enumerate() {
        fp[i] = u8::from_str_radix(part, 16).map_err(|_| "invalid hex byte")?;
    }
    Ok(fp)
}

/// Generate a 6-digit mutual pairing PIN by hashing the combination of client and server fingerprints.
pub fn generate_pairing_pin(client_fp: &[u8; 32], server_fp: &[u8; 32]) -> u32 {
    let mut hasher = Sha256::new();
    // Sort fingerprints to guarantee the PIN is symmetric on both client and server sides
    if client_fp <= server_fp {
        hasher.update(client_fp);
        hasher.update(server_fp);
    } else {
        hasher.update(server_fp);
        hasher.update(client_fp);
    }
    let result = hasher.finalize();
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&result[0..4]);
    // Map to a 6-digit number
    u32::from_be_bytes(bytes) % 1_000_000
}

/// State tracking for pending certificate validation during pairing.
#[derive(Debug, Clone, Default)]
pub struct PendingPairingState {
    /// The fingerprint of the certificate presented by the peer during the handshake.
    pub peer_fingerprint: Option<[u8; 32]>,
}

/// Custom verifier for checking server certificates via TOFU.
#[derive(Debug)]
pub struct TofuServerVerifier {
    /// Set of trusted server fingerprints.
    trusted_fingerprints: Arc<Mutex<HashSet<[u8; 32]>>>,
    /// Whether we are currently in pairing mode (accepts any cert temporarily).
    is_pairing: bool,
    /// Shareable slot to write the verified peer fingerprint during pairing.
    pending_state: Option<Arc<Mutex<PendingPairingState>>>,
    /// Cryptographic algorithms supported by default provider.
    supported_algos: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl TofuServerVerifier {
    pub fn new(
        trusted_fingerprints: Arc<Mutex<HashSet<[u8; 32]>>>,
        is_pairing: bool,
        pending_state: Option<Arc<Mutex<PendingPairingState>>>,
    ) -> Self {
        let supported_algos =
            rustls::crypto::ring::default_provider().signature_verification_algorithms;
        Self {
            trusted_fingerprints,
            is_pairing,
            pending_state,
            supported_algos,
        }
    }
}

impl ServerCertVerifier for TofuServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let fp = compute_fingerprint(end_entity.as_ref());

        if self.is_pairing {
            // In pairing mode, accept any certificate but record the fingerprint.
            if let Some(ref pending) = self.pending_state {
                let mut state = pending.lock().unwrap();
                state.peer_fingerprint = Some(fp);
            }
            return Ok(ServerCertVerified::assertion());
        }

        // Normal mode: only accept if fingerprint matches the pinned store.
        let trusted = self.trusted_fingerprints.lock().unwrap();
        if trusted.contains(&fp) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.supported_algos)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported_algos)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported_algos.supported_schemes()
    }
}

/// Custom verifier for checking client certificates via TOFU.
#[derive(Debug)]
pub struct TofuClientVerifier {
    /// Set of trusted client fingerprints.
    trusted_fingerprints: Arc<Mutex<HashSet<[u8; 32]>>>,
    /// Whether we are currently in pairing mode (accepts any cert temporarily).
    is_pairing: bool,
    /// Shareable slot to write the verified peer fingerprint during pairing.
    pending_state: Option<Arc<Mutex<PendingPairingState>>>,
    /// Cryptographic algorithms supported by default provider.
    supported_algos: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl TofuClientVerifier {
    pub fn new(
        trusted_fingerprints: Arc<Mutex<HashSet<[u8; 32]>>>,
        is_pairing: bool,
        pending_state: Option<Arc<Mutex<PendingPairingState>>>,
    ) -> Self {
        let supported_algos =
            rustls::crypto::ring::default_provider().signature_verification_algorithms;
        Self {
            trusted_fingerprints,
            is_pairing,
            pending_state,
            supported_algos,
        }
    }
}

impl ClientCertVerifier for TofuClientVerifier {
    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        let fp = compute_fingerprint(end_entity.as_ref());

        if self.is_pairing {
            // In pairing mode, accept any certificate but record the fingerprint.
            if let Some(ref pending) = self.pending_state {
                let mut state = pending.lock().unwrap();
                state.peer_fingerprint = Some(fp);
            }
            return Ok(ClientCertVerified::assertion());
        }

        // Normal mode: only accept if fingerprint matches the pinned store.
        let trusted = self.trusted_fingerprints.lock().unwrap();
        if trusted.contains(&fp) {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.supported_algos)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported_algos)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported_algos.supported_schemes()
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_calculations() {
        let dummy_cert = b"my-dummy-certificate-der-bytes";
        let fp = compute_fingerprint(dummy_cert);
        let fp_str = fingerprint_to_string(&fp);
        assert_eq!(fp_str.split(':').count(), 32);

        let parsed = string_to_fingerprint(&fp_str).unwrap();
        assert_eq!(fp, parsed);
    }

    #[test]
    fn pin_generation_is_symmetric() {
        let client_fp = [1u8; 32];
        let server_fp = [2u8; 32];

        let pin1 = generate_pairing_pin(&client_fp, &server_fp);
        let pin2 = generate_pairing_pin(&server_fp, &client_fp);

        assert_eq!(pin1, pin2);
        assert!(pin1 < 1_000_000);
    }

    #[test]
    fn test_mtls_tofu_handshake() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        // 1. Generate keys and certs for client and server.
        let rcgen::CertifiedKey {
            cert: server_cert,
            signing_key: server_signing_key,
        } = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let server_cert_der = CertificateDer::from(server_cert.der().to_vec());
        let server_key_der =
            rustls_pki_types::PrivatePkcs8KeyDer::from(server_signing_key.serialize_der());

        let rcgen::CertifiedKey {
            cert: client_cert,
            signing_key: client_signing_key,
        } = rcgen::generate_simple_self_signed(vec!["client".to_string()]).unwrap();
        let client_cert_der = CertificateDer::from(client_cert.der().to_vec());
        let client_key_der =
            rustls_pki_types::PrivatePkcs8KeyDer::from(client_signing_key.serialize_der());

        // 2. Set up TOFU verifiers in pairing mode.
        let server_trusted = Arc::new(Mutex::new(HashSet::new()));
        let server_pending = Arc::new(Mutex::new(PendingPairingState::default()));
        let server_verifier = Arc::new(TofuClientVerifier::new(
            server_trusted.clone(),
            true, // pairing mode
            Some(server_pending.clone()),
        ));

        let client_trusted = Arc::new(Mutex::new(HashSet::new()));
        let client_pending = Arc::new(Mutex::new(PendingPairingState::default()));
        let client_verifier = Arc::new(TofuServerVerifier::new(
            client_trusted.clone(),
            true, // pairing mode
            Some(client_pending.clone()),
        ));

        // 3. Configure TLS.
        let server_config = rustls::ServerConfig::builder()
            .with_client_cert_verifier(server_verifier)
            .with_single_cert(vec![server_cert_der.clone()], server_key_der.into())
            .unwrap();

        let client_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(client_verifier)
            .with_client_auth_cert(vec![client_cert_der.clone()], client_key_der.into())
            .unwrap();

        // 4. Run Handshake.
        let mut server_conn = rustls::ServerConnection::new(Arc::new(server_config)).unwrap();
        let mut client_conn = rustls::ClientConnection::new(
            Arc::new(client_config),
            ServerName::try_from("localhost").unwrap(),
        )
        .unwrap();

        // Feed buffers back and forth.
        let mut client_to_server = vec![0u8; 4096];
        let mut server_to_client = vec![0u8; 4096];

        loop {
            // Client writes to server
            if client_conn.wants_write() {
                let n = client_conn
                    .write_tls(&mut client_to_server.as_mut_slice())
                    .unwrap();
                if n > 0 {
                    let mut read_cursor = std::io::Cursor::new(&client_to_server[..n]);
                    server_conn.read_tls(&mut read_cursor).unwrap();
                    server_conn.process_new_packets().unwrap();
                }
            }

            // Server writes to client
            if server_conn.wants_write() {
                let n = server_conn
                    .write_tls(&mut server_to_client.as_mut_slice())
                    .unwrap();
                if n > 0 {
                    let mut read_cursor = std::io::Cursor::new(&server_to_client[..n]);
                    client_conn.read_tls(&mut read_cursor).unwrap();
                    client_conn.process_new_packets().unwrap();
                }
            }

            if !client_conn.is_handshaking() && !server_conn.is_handshaking() {
                break;
            }
        }

        // Verify fingerprints captured.
        let client_fp_captured = server_pending.lock().unwrap().peer_fingerprint.unwrap();
        let server_fp_captured = client_pending.lock().unwrap().peer_fingerprint.unwrap();

        assert_eq!(
            client_fp_captured,
            compute_fingerprint(client_cert_der.as_ref())
        );
        assert_eq!(
            server_fp_captured,
            compute_fingerprint(server_cert_der.as_ref())
        );

        // Verify pairing PIN.
        let pin = generate_pairing_pin(&client_fp_captured, &server_fp_captured);
        assert!(pin < 1_000_000);

        // 5. Test Normal Mode: pairing confirmed.
        server_trusted.lock().unwrap().insert(client_fp_captured);
        client_trusted.lock().unwrap().insert(server_fp_captured);

        // Re-configure verifiers in normal mode.
        let server_verifier_normal = Arc::new(TofuClientVerifier::new(
            server_trusted.clone(),
            false, // normal mode
            None,
        ));
        let client_verifier_normal = Arc::new(TofuServerVerifier::new(
            client_trusted.clone(),
            false, // normal mode
            None,
        ));

        let server_config_normal = rustls::ServerConfig::builder()
            .with_client_cert_verifier(server_verifier_normal)
            .with_single_cert(
                vec![server_cert_der.clone()],
                rustls_pki_types::PrivatePkcs8KeyDer::from(server_signing_key.serialize_der())
                    .into(),
            )
            .unwrap();

        let client_config_normal = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(client_verifier_normal)
            .with_client_auth_cert(
                vec![client_cert_der.clone()],
                rustls_pki_types::PrivatePkcs8KeyDer::from(client_signing_key.serialize_der())
                    .into(),
            )
            .unwrap();

        let mut server_conn_normal =
            rustls::ServerConnection::new(Arc::new(server_config_normal)).unwrap();
        let mut client_conn_normal = rustls::ClientConnection::new(
            Arc::new(client_config_normal),
            ServerName::try_from("localhost").unwrap(),
        )
        .unwrap();

        loop {
            if client_conn_normal.wants_write() {
                let n = client_conn_normal
                    .write_tls(&mut client_to_server.as_mut_slice())
                    .unwrap();
                if n > 0 {
                    let mut read_cursor = std::io::Cursor::new(&client_to_server[..n]);
                    server_conn_normal.read_tls(&mut read_cursor).unwrap();
                    server_conn_normal.process_new_packets().unwrap();
                }
            }

            if server_conn_normal.wants_write() {
                let n = server_conn_normal
                    .write_tls(&mut server_to_client.as_mut_slice())
                    .unwrap();
                if n > 0 {
                    let mut read_cursor = std::io::Cursor::new(&server_to_client[..n]);
                    client_conn_normal.read_tls(&mut read_cursor).unwrap();
                    client_conn_normal.process_new_packets().unwrap();
                }
            }

            if !client_conn_normal.is_handshaking() && !server_conn_normal.is_handshaking() {
                break;
            }
        }

        // 6. Test Untrusted Client connection rejection.
        let rcgen::CertifiedKey {
            cert: untrusted_client_cert,
            signing_key: untrusted_client_signing_key,
        } = rcgen::generate_simple_self_signed(vec!["untrusted".to_string()]).unwrap();
        let untrusted_client_cert_der = CertificateDer::from(untrusted_client_cert.der().to_vec());

        let untrusted_client_config = rustls::ClientConfig::builder()
            .dangerous()
            // Reuse normal verifier (trusts only the first server)
            .with_custom_certificate_verifier(Arc::new(TofuServerVerifier::new(
                client_trusted.clone(),
                false,
                None,
            )))
            .with_client_auth_cert(
                vec![untrusted_client_cert_der.clone()],
                rustls_pki_types::PrivatePkcs8KeyDer::from(
                    untrusted_client_signing_key.serialize_der(),
                )
                .into(),
            )
            .unwrap();

        let mut server_conn_reject = rustls::ServerConnection::new(Arc::new(
            rustls::ServerConfig::builder()
                .with_client_cert_verifier(Arc::new(TofuClientVerifier::new(
                    server_trusted.clone(),
                    false, // normal mode
                    None,
                )))
                .with_single_cert(
                    vec![server_cert_der.clone()],
                    rustls_pki_types::PrivatePkcs8KeyDer::from(server_signing_key.serialize_der())
                        .into(),
                )
                .unwrap(),
        ))
        .unwrap();

        let mut client_conn_reject = rustls::ClientConnection::new(
            Arc::new(untrusted_client_config),
            ServerName::try_from("localhost").unwrap(),
        )
        .unwrap();

        let mut failed = false;
        loop {
            if client_conn_reject.wants_write() {
                let n = client_conn_reject
                    .write_tls(&mut client_to_server.as_mut_slice())
                    .unwrap();
                if n > 0 {
                    let mut read_cursor = std::io::Cursor::new(&client_to_server[..n]);
                    if server_conn_reject.read_tls(&mut read_cursor).is_err() {
                        failed = true;
                        break;
                    }
                    if server_conn_reject.process_new_packets().is_err() {
                        failed = true;
                        break;
                    }
                }
            }

            if server_conn_reject.wants_write() {
                let n = server_conn_reject
                    .write_tls(&mut server_to_client.as_mut_slice())
                    .unwrap();
                if n > 0 {
                    let mut read_cursor = std::io::Cursor::new(&server_to_client[..n]);
                    if client_conn_reject.read_tls(&mut read_cursor).is_err() {
                        failed = true;
                        break;
                    }
                    if client_conn_reject.process_new_packets().is_err() {
                        failed = true;
                        break;
                    }
                }
            }

            if !client_conn_reject.is_handshaking() && !server_conn_reject.is_handshaking() {
                break;
            }
        }

        // If handshakes completed without error, check if it failed due to alert or error.
        if !failed {
            // Handshake should have been rejected during cert check.
            assert!(server_conn_reject.peer_certificates().is_none());
        }
    }
}
