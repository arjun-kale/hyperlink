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
    /// Per-feature enable/disable toggles and bandwidth limit (Phase 11 preferences).
    /// `#[serde(default)]` so config files written before this field existed keep loading.
    #[serde(default)]
    pub preferences: HostPreferences,
}

/// User-configurable feature toggles and limits, edited via the Libadwaita
/// preferences window and read at connection/session start.
///
/// These are intentionally *not* hot-reloaded into an already-open connection —
/// a change here takes effect the next time a session starts, which is called
/// out in the preferences window UI rather than silently assumed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostPreferences {
    /// Mirror video from the phone.
    pub enable_video: bool,
    /// Accept keyboard/mouse/touch input events destined for the phone.
    pub enable_input: bool,
    /// Mirror phone notifications to the desktop.
    pub enable_notifications: bool,
    /// Sync the system clipboard in both directions.
    pub enable_clipboard: bool,
    /// Mount the phone's storage as a local virtual filesystem.
    pub enable_file_access: bool,
    /// Opportunistically pre-warm reconnects based on proximity signals.
    pub enable_proximity_prewarm: bool,
    /// Cap the video encoder's target bitrate. `None` means no cap (encoder default).
    pub max_bitrate_kbps: Option<u32>,
    /// Write local, redacted crash reports on panic (see `docs/SECURITY_REVIEW.md`
    /// for exactly what is and isn't included). Never transmitted anywhere —
    /// this only controls whether a report is written to disk for the user's
    /// own troubleshooting / to attach to a bug report themselves.
    pub crash_reporting_enabled: bool,
    /// Periodically check (not auto-install) whether a newer release exists.
    pub update_check_enabled: bool,
}

impl Default for HostPreferences {
    fn default() -> Self {
        Self {
            enable_video: true,
            enable_input: true,
            enable_notifications: true,
            enable_clipboard: true,
            enable_file_access: true,
            enable_proximity_prewarm: true,
            max_bitrate_kbps: None,
            crash_reporting_enabled: true,
            update_check_enabled: true,
        }
    }
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
            preferences: HostPreferences::default(),
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
        self.trusted_peers
            .insert(peer_name.to_string(), fingerprint.to_string());
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
        assert_eq!(reloaded.preferences, HostPreferences::default());
    }

    /// A config.json written before `HostPreferences` existed (Phase 0-10) must still
    /// load — `#[serde(default)]` on `preferences` is what makes that true.
    #[test]
    fn pre_phase11_config_without_preferences_field_still_loads() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");

        let legacy_json = r#"{
            "device_name": "old-desktop",
            "cert_pem": "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
            "key_pem": "-----BEGIN PRIVATE KEY-----\nMIGH\n-----END PRIVATE KEY-----\n",
            "trusted_peers": {"phone-1": "AA:BB"}
        }"#;
        fs::write(&path, legacy_json).unwrap();

        let loaded = DeviceConfig::load_or_create(&path, "ignored-name").unwrap();
        assert_eq!(loaded.device_name, "old-desktop");
        assert_eq!(loaded.trusted_peers.get("phone-1").unwrap(), "AA:BB");
        // Missing field falls back to defaults rather than failing to parse.
        assert_eq!(loaded.preferences, HostPreferences::default());
        assert!(loaded.preferences.crash_reporting_enabled);
    }
}
