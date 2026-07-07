//! Persistent configuration storage for paired certificates and trusted peer fingerprints.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Host or client identity configuration and list of paired peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Device display name.
    pub device_name: String,
    /// PEM-encoded self-signed TLS certificate.
    pub cert_pem: String,
    /// PEM-encoded private key corresponding to the certificate.
    pub key_pem: String,
    /// Trusted peer fingerprints map (device_name -> hex fingerprint).
    pub trusted_peers: HashMap<String, String>,
}

impl DeviceConfig {
    /// Load existing config from a file path, or generate a new one if missing.
    pub fn load_or_create(path: &Path, default_name: &str) -> anyhow::Result<Self> {
        if path.exists() {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            let config: Self = serde_json::from_reader(reader)?;
            Ok(config)
        } else {
            // Generate self-signed certificate and key.
            let config = Self::generate(default_name)?;
            config.save(path)?;
            Ok(config)
        }
    }

    /// Generate new cert and private key pair for the device.
    pub fn generate(device_name: &str) -> anyhow::Result<Self> {
        let subject_alt_names = vec![device_name.to_string(), "localhost".to_string()];
        
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(subject_alt_names)?;

        Ok(Self {
            device_name: device_name.to_string(),
            cert_pem: cert.pem(),
            key_pem: signing_key.serialize_pem(),
            trusted_peers: HashMap::new(),
        })
    }

    /// Save current configuration state to a file.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let mut file = File::create(path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    /// Add a peer to the trusted list and save.
    pub fn add_trusted_peer(&mut self, peer_name: &str, fingerprint: &str) {
        self.trusted_peers.insert(peer_name.to_string(), fingerprint.to_string());
    }

    /// Remove a peer from the trusted list.
    pub fn remove_trusted_peer(&mut self, peer_name: &str) {
        self.trusted_peers.remove(peer_name);
    }

    /// Get the set of 32-byte trusted fingerprints.
    pub fn get_trusted_fingerprints_set(&self) -> HashSet<[u8; 32]> {
        let mut set = HashSet::new();
        for fp_str in self.trusted_peers.values() {
            if let Ok(fp) = crate::crypto::string_to_fingerprint(fp_str) {
                set.insert(fp);
            }
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn config_lifecycle() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");

        // 1. Create/Load default
        let mut config = DeviceConfig::load_or_create(&path, "test-device").unwrap();
        assert_eq!(config.device_name, "test-device");
        assert!(!config.cert_pem.is_empty());
        assert!(!config.key_pem.is_empty());
        assert!(config.trusted_peers.is_empty());

        // 2. Modify and Save
        config.add_trusted_peer("peer-1", "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF");
        config.save(&path).unwrap();

        // 3. Reload
        let reloaded = DeviceConfig::load_or_create(&path, "ignored-name").unwrap();
        assert_eq!(reloaded.device_name, "test-device");
        assert_eq!(reloaded.trusted_peers.get("peer-1").unwrap(), "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF");
    }
}
