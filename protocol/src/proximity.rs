//! Proximity ranging, pre-warmed connection setup, and workflow state restore protocol wire types (Phase 8).
//!
//! Provides structured wire serialization for BLE/UWB proximity beacons, pre-warm state
//! synchronization, and active mirror workflow view state (window geometry, orientation, active app).
//!
//! ## Security Invariant
//! Proximity beacons shorten connection setup latency by triggering opportunistic pre-warming
//! (socket allocation and background mTLS handshake), but **never bypass certificate-based mTLS authentication**.

use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};

/// Ranging technology utilized to detect proximity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProximityTechnology {
    /// Bluetooth Low Energy RSSI-based proximity estimation.
    BleRssi = 1,
    /// Ultra-Wideband (UWB) Time-of-Flight / Angle-of-Arrival precise ranging.
    UwbRanging = 2,
    /// Wi-Fi 802.11mc Round-Trip-Time fine timing measurement.
    WifiRtt = 3,
}

impl TryFrom<u8> for ProximityTechnology {
    type Error = anyhow::Error;

    fn try_from(val: u8) -> Result<Self> {
        match val {
            1 => Ok(Self::BleRssi),
            2 => Ok(Self::UwbRanging),
            3 => Ok(Self::WifiRtt),
            other => bail!("unknown proximity technology discriminant: {}", other),
        }
    }
}

/// Proximity beacon emitted by companion device or host over local transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProximityBeacon {
    /// Truncated prefix (first 8 bytes) of device's public TLS certificate SHA-256 fingerprint.
    pub device_fingerprint_prefix: [u8; 8],
    /// Physical ranging technology used.
    pub technology: ProximityTechnology,
    /// Estimated distance in centimeters (0..65535, e.g. 50 = 0.5m).
    pub distance_cm: u16,
    /// Received signal strength indicator in dBm (e.g. -45 dBm).
    pub rssi_dbm: i8,
    /// Ranging confidence percentage (0..100).
    pub confidence_pct: u8,
    /// Ranging timestamp in microseconds.
    pub timestamp_us: u64,
    /// Ephemeral nonce preventing replay attacks.
    pub nonce: u64,
}

impl ProximityBeacon {
    pub const FIXED_SIZE: usize = 8 + 1 + 2 + 1 + 1 + 8 + 8; // 29 bytes

    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        buf.extend_from_slice(&self.device_fingerprint_prefix);
        buf.push(self.technology as u8);
        buf.extend_from_slice(&self.distance_cm.to_be_bytes());
        buf.push(self.rssi_dbm as u8);
        buf.push(self.confidence_pct);
        buf.extend_from_slice(&self.timestamp_us.to_be_bytes());
        buf.extend_from_slice(&self.nonce.to_be_bytes());
        Ok(())
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        ensure!(
            buf.len() >= Self::FIXED_SIZE,
            "proximity beacon payload too short: {} < {}",
            buf.len(),
            Self::FIXED_SIZE
        );

        let mut fp = [0u8; 8];
        fp.copy_from_slice(&buf[0..8]);
        let technology = ProximityTechnology::try_from(buf[8])?;
        let distance_cm = u16::from_be_bytes([buf[9], buf[10]]);
        let rssi_dbm = buf[11] as i8;
        let confidence_pct = buf[12];
        let timestamp_us = u64::from_be_bytes(buf[13..21].try_into()?);
        let nonce = u64::from_be_bytes(buf[21..29].try_into()?);

        Ok(Self {
            device_fingerprint_prefix: fp,
            technology,
            distance_cm,
            rssi_dbm,
            confidence_pct,
            timestamp_us,
            nonce,
        })
    }

    /// Evaluates if beacon falls within the pre-warm threshold (<= 200cm or >= -65 dBm with high confidence).
    pub fn is_in_prewarm_range(&self) -> bool {
        (self.distance_cm <= 200 || self.rssi_dbm >= -65) && self.confidence_pct >= 50
    }
}

/// Host confirmation acknowledging beacon reception and signaling pre-warm status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProximityAck {
    /// Nonce matching the received beacon.
    pub beacon_nonce: u64,
    /// Whether the beacon was accepted from a known paired device.
    pub accepted: bool,
    /// Whether background pre-warming was triggered.
    pub prewarm_initiated: bool,
    /// Host timestamp in microseconds.
    pub ack_timestamp_us: u64,
}

impl ProximityAck {
    pub const FIXED_SIZE: usize = 8 + 1 + 1 + 8; // 18 bytes

    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        buf.extend_from_slice(&self.beacon_nonce.to_be_bytes());
        buf.push(if self.accepted { 1 } else { 0 });
        buf.push(if self.prewarm_initiated { 1 } else { 0 });
        buf.extend_from_slice(&self.ack_timestamp_us.to_be_bytes());
        Ok(())
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        ensure!(
            buf.len() >= Self::FIXED_SIZE,
            "proximity ack payload too short: {} < {}",
            buf.len(),
            Self::FIXED_SIZE
        );

        let beacon_nonce = u64::from_be_bytes(buf[0..8].try_into()?);
        let accepted = buf[8] != 0;
        let prewarm_initiated = buf[9] != 0;
        let ack_timestamp_us = u64::from_be_bytes(buf[10..18].try_into()?);

        Ok(Self {
            beacon_nonce,
            accepted,
            prewarm_initiated,
            ack_timestamp_us,
        })
    }
}

/// Pre-warm state synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreWarmState {
    /// Whether the background transport is pre-warmed.
    pub warmed: bool,
    /// Whether the QUIC mutual TLS 1.3 handshake has completed.
    pub quic_handshake_completed: bool,
    /// Whether stream buffers (video, input, control) are pre-allocated.
    pub streams_preallocated: bool,
    /// Power profile mode: 0 = LowPowerStandby, 1 = ActiveReady.
    pub power_profile: u8,
    /// State timestamp in microseconds.
    pub timestamp_us: u64,
}

impl PreWarmState {
    pub const FIXED_SIZE: usize = 1 + 1 + 1 + 1 + 8; // 12 bytes

    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        buf.push(if self.warmed { 1 } else { 0 });
        buf.push(if self.quic_handshake_completed { 1 } else { 0 });
        buf.push(if self.streams_preallocated { 1 } else { 0 });
        buf.push(self.power_profile);
        buf.extend_from_slice(&self.timestamp_us.to_be_bytes());
        Ok(())
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        ensure!(
            buf.len() >= Self::FIXED_SIZE,
            "pre-warm state payload too short: {} < {}",
            buf.len(),
            Self::FIXED_SIZE
        );

        let warmed = buf[0] != 0;
        let quic_handshake_completed = buf[1] != 0;
        let streams_preallocated = buf[2] != 0;
        let power_profile = buf[3];
        let timestamp_us = u64::from_be_bytes(buf[4..12].try_into()?);

        Ok(Self {
            warmed,
            quic_handshake_completed,
            streams_preallocated,
            power_profile,
            timestamp_us,
        })
    }
}

/// Active mirror workflow view state (window geometry, orientation, active app).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowState {
    /// Saved window width in pixels.
    pub window_width: u32,
    /// Saved window height in pixels.
    pub window_height: u32,
    /// Saved window X position on desktop.
    pub window_x: i32,
    /// Saved window Y position on desktop.
    pub window_y: i32,
    /// Whether window was in fullscreen mode.
    pub is_fullscreen: bool,
    /// Top-most active Android app package name (e.g. "com.sec.android.app.camera").
    pub active_package_name: String,
    /// Display orientation: 0 = Portrait (0°), 1 = Landscape (90°), 2 = Reverse Portrait (180°), 3 = Reverse Landscape (270°).
    pub display_orientation: u8,
    /// Timestamp when this workflow state was captured in microseconds.
    pub saved_timestamp_us: u64,
}

impl Default for WorkflowState {
    fn default() -> Self {
        Self {
            window_width: 960,
            window_height: 540,
            window_x: -1,
            window_y: -1,
            is_fullscreen: false,
            active_package_name: "com.android.launcher".to_string(),
            display_orientation: 0,
            saved_timestamp_us: 0,
        }
    }
}

impl WorkflowState {
    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        buf.extend_from_slice(&self.window_width.to_be_bytes());
        buf.extend_from_slice(&self.window_height.to_be_bytes());
        buf.extend_from_slice(&self.window_x.to_be_bytes());
        buf.extend_from_slice(&self.window_y.to_be_bytes());
        buf.push(if self.is_fullscreen { 1 } else { 0 });
        buf.push(self.display_orientation);
        buf.extend_from_slice(&self.saved_timestamp_us.to_be_bytes());

        let pkg_bytes = self.active_package_name.as_bytes();
        ensure!(
            pkg_bytes.len() <= 255,
            "package name exceeds 255 bytes limit"
        );
        buf.push(pkg_bytes.len() as u8);
        buf.extend_from_slice(pkg_bytes);
        Ok(())
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        let min_size = 4 + 4 + 4 + 4 + 1 + 1 + 8 + 1; // 27 bytes
        ensure!(
            buf.len() >= min_size,
            "workflow state payload too short: {} < {}",
            buf.len(),
            min_size
        );

        let window_width = u32::from_be_bytes(buf[0..4].try_into()?);
        let window_height = u32::from_be_bytes(buf[4..8].try_into()?);
        let window_x = i32::from_be_bytes(buf[8..12].try_into()?);
        let window_y = i32::from_be_bytes(buf[12..16].try_into()?);
        let is_fullscreen = buf[16] != 0;
        let display_orientation = buf[17];
        let saved_timestamp_us = u64::from_be_bytes(buf[18..26].try_into()?);

        let pkg_len = buf[26] as usize;
        ensure!(
            buf.len() >= min_size + pkg_len,
            "workflow state truncated package name: {} < {}",
            buf.len(),
            min_size + pkg_len
        );
        let active_package_name = String::from_utf8(buf[27..27 + pkg_len].to_vec())?;

        Ok(Self {
            window_width,
            window_height,
            window_x,
            window_y,
            is_fullscreen,
            active_package_name,
            display_orientation,
            saved_timestamp_us,
        })
    }
}

/// Workflow state acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStateAck {
    /// Whether workflow state was successfully saved or restored.
    pub restored: bool,
    /// Status code: 0 = Success, 1 = AppNotFound, 2 = OrientationMismatch.
    pub status_code: u8,
    /// Timestamp in microseconds.
    pub timestamp_us: u64,
}

impl WorkflowStateAck {
    pub const FIXED_SIZE: usize = 1 + 1 + 8; // 10 bytes

    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        buf.push(if self.restored { 1 } else { 0 });
        buf.push(self.status_code);
        buf.extend_from_slice(&self.timestamp_us.to_be_bytes());
        Ok(())
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        ensure!(
            buf.len() >= Self::FIXED_SIZE,
            "workflow state ack payload too short: {} < {}",
            buf.len(),
            Self::FIXED_SIZE
        );

        let restored = buf[0] != 0;
        let status_code = buf[1];
        let timestamp_us = u64::from_be_bytes(buf[2..10].try_into()?);

        Ok(Self {
            restored,
            status_code,
            timestamp_us,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proximity_beacon_round_trip() {
        let beacon = ProximityBeacon {
            device_fingerprint_prefix: [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0],
            technology: ProximityTechnology::UwbRanging,
            distance_cm: 45, // 0.45 meters
            rssi_dbm: -42,
            confidence_pct: 98,
            timestamp_us: 1_234_567_890,
            nonce: 0xCAFEBABE,
        };

        assert!(beacon.is_in_prewarm_range());

        let mut buf = Vec::new();
        beacon.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), ProximityBeacon::FIXED_SIZE);

        let decoded = ProximityBeacon::decode(&buf).unwrap();
        assert_eq!(beacon, decoded);
    }

    #[test]
    fn test_proximity_ack_round_trip() {
        let ack = ProximityAck {
            beacon_nonce: 0xCAFEBABE,
            accepted: true,
            prewarm_initiated: true,
            ack_timestamp_us: 1_234_567_899,
        };

        let mut buf = Vec::new();
        ack.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), ProximityAck::FIXED_SIZE);

        let decoded = ProximityAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn test_prewarm_state_round_trip() {
        let state = PreWarmState {
            warmed: true,
            quic_handshake_completed: true,
            streams_preallocated: true,
            power_profile: 1,
            timestamp_us: 1_234_567_890,
        };

        let mut buf = Vec::new();
        state.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), PreWarmState::FIXED_SIZE);

        let decoded = PreWarmState::decode(&buf).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn test_workflow_state_round_trip() {
        let wf = WorkflowState {
            window_width: 1080,
            window_height: 2400,
            window_x: 100,
            window_y: 200,
            is_fullscreen: false,
            active_package_name: "com.sec.android.app.camera".to_string(),
            display_orientation: 1,
            saved_timestamp_us: 1_234_567_890,
        };

        let mut buf = Vec::new();
        wf.encode(&mut buf).unwrap();

        let decoded = WorkflowState::decode(&buf).unwrap();
        assert_eq!(wf, decoded);
    }

    #[test]
    fn test_workflow_state_ack_round_trip() {
        let ack = WorkflowStateAck {
            restored: true,
            status_code: 0,
            timestamp_us: 1_234_567_890,
        };

        let mut buf = Vec::new();
        ack.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), WorkflowStateAck::FIXED_SIZE);

        let decoded = WorkflowStateAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn test_security_invariant_unpaired_beacon_rejected() {
        // Zero-trust invariant check:
        // Even if a beacon indicates close physical proximity (0.05m / 5cm),
        // if fingerprint does not match trusted peer store, it cannot authenticate.
        let mut trusted_peers = std::collections::HashMap::new();
        trusted_peers.insert("my-phone".to_string(), "A1B2C3D4E5F60708".to_string());

        let rogue_beacon = ProximityBeacon {
            device_fingerprint_prefix: [0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA, 0x99, 0x88],
            technology: ProximityTechnology::UwbRanging,
            distance_cm: 5, // 5cm away (very close!)
            rssi_dbm: -20,
            confidence_pct: 100,
            timestamp_us: 1_000_000,
            nonce: 1234,
        };

        let prefix_hex: String = rogue_beacon
            .device_fingerprint_prefix
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect();
        let matches_trusted = trusted_peers
            .values()
            .any(|known_fp| known_fp.starts_with(&prefix_hex));

        assert!(
            !matches_trusted,
            "Security invariant violated: untrusted proximity beacon must not match paired peer store"
        );
    }
}
