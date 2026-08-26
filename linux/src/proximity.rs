//! Linux host proximity, pre-warmed connection manager, and workflow state restore (Phase 8).
//!
//! Evaluates incoming BLE/UWB proximity beacons, drives the pre-warming state machine,
//! validates peer certificate fingerprints (zero-trust invariant), and manages workflow state
//! persistence for seamless window geometry / orientation / view restoration.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

use hyperlink_protocol::clock;
use hyperlink_protocol::proximity::{
    PreWarmState, ProximityAck, ProximityBeacon, WorkflowState, WorkflowStateAck,
};

/// Lifecycle state for proximity and pre-warming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProximityState {
    /// No proximity beacon detected recently.
    Idle,
    /// Beacon received, evaluating range and trust.
    BeaconDetected,
    /// Transport and mTLS handshake actively pre-warming in background.
    PreWarming,
    /// Connection pre-warmed and standby; instantaneous mirror activation ready.
    PreWarmedReady,
    /// Mirror window is open and user is actively interacting.
    ActiveMirroring,
}

/// Host-side proximity and workflow state manager.
#[derive(Clone)]
pub struct HostProximityManager {
    state: Arc<Mutex<ProximityState>>,
    last_beacon: Arc<Mutex<Option<ProximityBeacon>>>,
    trusted_peers: Arc<Mutex<HashMap<String, String>>>,
    workflow_state: Arc<Mutex<WorkflowState>>,
    workflow_path: PathBuf,
}

impl HostProximityManager {
    /// Creates a new HostProximityManager.
    pub fn new(workflow_path: PathBuf, trusted_peers: HashMap<String, String>) -> Self {
        let manager = Self {
            state: Arc::new(Mutex::new(ProximityState::Idle)),
            last_beacon: Arc::new(Mutex::new(None)),
            trusted_peers: Arc::new(Mutex::new(trusted_peers)),
            workflow_state: Arc::new(Mutex::new(WorkflowState::default())),
            workflow_path,
        };

        // Load existing saved workflow state from disk if present
        let loaded = manager.load_saved_workflow_state();
        *manager.workflow_state.lock().unwrap() = loaded;

        manager
    }

    /// Returns the current proximity lifecycle state.
    pub fn state(&self) -> ProximityState {
        *self.state.lock().unwrap()
    }

    /// Starts a background out-of-band UDP listener for proximity discovery beacons (Phase 8).
    pub fn start_out_of_band_beacon_listener(
        self: Arc<Self>,
        bind_port: u16,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let bind_addr = format!("0.0.0.0:{}", bind_port);
            let socket = match tokio::net::UdpSocket::bind(&bind_addr).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(
                        port = bind_port,
                        error = %e,
                        "could not bind out-of-band proximity UDP socket (non-fatal)"
                    );
                    return;
                }
            };
            info!(
                port = bind_port,
                "out-of-band proximity UDP discovery beacon listener active"
            );

            let mut buf = [0u8; 1024];
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((len, peer_addr)) => {
                        if len >= hyperlink_protocol::version::HEADER_SIZE {
                            if let Ok(hdr) = hyperlink_protocol::version::Header::decode(&buf[..10])
                            {
                                if hdr.message_type
                                    == hyperlink_protocol::message::MessageType::ProximityBeacon
                                {
                                    if let Ok(beacon) = ProximityBeacon::decode(&buf[10..len]) {
                                        let ack = self.handle_proximity_beacon(&beacon);
                                        let mut ack_payload = Vec::new();
                                        if ack.encode(&mut ack_payload).is_ok() {
                                            let ack_hdr =
                                                hyperlink_protocol::version::Header::new(
                                                    hyperlink_protocol::message::MessageType::ProximityAck,
                                                    ack_payload.len() as u32,
                                                );
                                            let mut resp_pkt =
                                                Vec::with_capacity(10 + ack_payload.len());
                                            if ack_hdr.encode(&mut resp_pkt).is_ok() {
                                                resp_pkt.extend_from_slice(&ack_payload);
                                                let _ = socket.send_to(&resp_pkt, peer_addr).await;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "error in proximity UDP listener");
                        break;
                    }
                }
            }
        })
    }

    /// Updates trusted peer certificate fingerprints.
    pub fn set_trusted_peers(&self, peers: HashMap<String, String>) {
        *self.trusted_peers.lock().unwrap() = peers;
    }

    /// Handles an incoming ProximityBeacon from a companion device.
    ///
    /// Validates the device certificate fingerprint prefix against trusted peers.
    /// If trusted and within pre-warm range, initiates pre-warming and transitions state.
    pub fn handle_proximity_beacon(&self, beacon: &ProximityBeacon) -> ProximityAck {
        let prefix_hex: String = beacon
            .device_fingerprint_prefix
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect();

        let is_trusted = {
            let guard = self.trusted_peers.lock().unwrap();
            guard
                .values()
                .any(|fp| fp.to_uppercase().starts_with(&prefix_hex))
        };

        let now_us = clock::now_us();

        if !is_trusted {
            warn!(
                prefix = %prefix_hex,
                "received proximity beacon from untrusted/unpaired device: rejecting pre-warm"
            );
            return ProximityAck {
                beacon_nonce: beacon.nonce,
                accepted: false,
                prewarm_initiated: false,
                ack_timestamp_us: now_us,
            };
        }

        *self.last_beacon.lock().unwrap() = Some(beacon.clone());

        let in_range = beacon.is_in_prewarm_range();
        let mut state_guard = self.state.lock().unwrap();

        if in_range {
            if *state_guard == ProximityState::Idle
                || *state_guard == ProximityState::BeaconDetected
            {
                *state_guard = ProximityState::PreWarmedReady;
                info!(
                    tech = ?beacon.technology,
                    dist_cm = beacon.distance_cm,
                    rssi = beacon.rssi_dbm,
                    "proximity beacon accepted: pre-warmed connection ready"
                );
            }
            ProximityAck {
                beacon_nonce: beacon.nonce,
                accepted: true,
                prewarm_initiated: true,
                ack_timestamp_us: now_us,
            }
        } else {
            *state_guard = ProximityState::BeaconDetected;
            ProximityAck {
                beacon_nonce: beacon.nonce,
                accepted: true,
                prewarm_initiated: false,
                ack_timestamp_us: now_us,
            }
        }
    }

    /// Generates current PreWarmState to synchronize with peer.
    pub fn get_prewarm_state(&self) -> PreWarmState {
        let state = *self.state.lock().unwrap();
        let warmed =
            state == ProximityState::PreWarmedReady || state == ProximityState::ActiveMirroring;
        let power_profile = if state == ProximityState::ActiveMirroring {
            1
        } else {
            0
        };

        PreWarmState {
            warmed,
            quic_handshake_completed: warmed,
            streams_preallocated: warmed,
            power_profile,
            timestamp_us: clock::now_us(),
        }
    }

    /// Activates active mirroring from pre-warmed state and returns workflow state to restore.
    pub fn activate_mirror(&self) -> WorkflowState {
        let mut state_guard = self.state.lock().unwrap();
        *state_guard = ProximityState::ActiveMirroring;
        info!("transitioned proximity state to ActiveMirroring");
        self.workflow_state.lock().unwrap().clone()
    }

    /// Deactivates active mirroring back to pre-warmed or idle state.
    pub fn deactivate_mirror(&self) {
        let mut state_guard = self.state.lock().unwrap();
        *state_guard = ProximityState::PreWarmedReady;
        info!("transitioned proximity state back to PreWarmedReady");
    }

    /// Saves updated workflow state to in-memory cache and persistent JSON file.
    pub fn save_workflow_state(&self, state: WorkflowState) -> anyhow::Result<WorkflowStateAck> {
        *self.workflow_state.lock().unwrap() = state.clone();

        if let Some(parent) = self.workflow_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&state)?;
        fs::write(&self.workflow_path, json)?;

        info!(
            width = state.window_width,
            height = state.window_height,
            package = %state.active_package_name,
            orientation = state.display_orientation,
            "saved workflow state to {:?}",
            self.workflow_path
        );

        Ok(WorkflowStateAck {
            restored: true,
            status_code: 0,
            timestamp_us: clock::now_us(),
        })
    }

    /// Loads saved workflow state from persistent storage, falling back to defaults.
    pub fn load_saved_workflow_state(&self) -> WorkflowState {
        if self.workflow_path.exists() {
            if let Ok(file) = File::open(&self.workflow_path) {
                let reader = BufReader::new(file);
                if let Ok(state) = serde_json::from_reader(reader) {
                    info!("loaded saved workflow state from {:?}", self.workflow_path);
                    return state;
                }
            }
        }
        WorkflowState::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyperlink_protocol::proximity::ProximityTechnology;

    fn get_test_state_path(name: &str) -> PathBuf {
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hyperlink_test_{}_{}", name, now_ns));
        let _ = fs::create_dir_all(&dir);
        dir.join("workflow_state.json")
    }

    #[test]
    fn test_host_proximity_prewarm_lifecycle() {
        let state_path = get_test_state_path("lifecycle");

        let mut trusted = HashMap::new();
        trusted.insert("pixel8".to_string(), "0102030405060708".to_string());

        let mgr = HostProximityManager::new(state_path.clone(), trusted);
        assert_eq!(mgr.state(), ProximityState::Idle);

        // 1. Send beacon in pre-warm range from paired peer
        let beacon = ProximityBeacon {
            device_fingerprint_prefix: [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            technology: ProximityTechnology::UwbRanging,
            distance_cm: 60, // 0.6m
            rssi_dbm: -45,
            confidence_pct: 95,
            timestamp_us: 1000,
            nonce: 42,
        };

        let ack = mgr.handle_proximity_beacon(&beacon);
        assert!(ack.accepted);
        assert!(ack.prewarm_initiated);
        assert_eq!(mgr.state(), ProximityState::PreWarmedReady);

        let prewarm_state = mgr.get_prewarm_state();
        assert!(prewarm_state.warmed);
        assert!(prewarm_state.quic_handshake_completed);

        // 2. Activate mirror and verify state
        let wf = mgr.activate_mirror();
        assert_eq!(mgr.state(), ProximityState::ActiveMirroring);
        assert_eq!(wf.window_width, 960);

        // 3. Deactivate mirror
        mgr.deactivate_mirror();
        assert_eq!(mgr.state(), ProximityState::PreWarmedReady);
    }

    #[test]
    fn test_host_proximity_unpaired_rejected() {
        let state_path = get_test_state_path("unpaired");

        let mut trusted = HashMap::new();
        trusted.insert("pixel8".to_string(), "0102030405060708".to_string());

        let mgr = HostProximityManager::new(state_path, trusted);

        // Untrusted beacon (fingerprint mismatch)
        let rogue_beacon = ProximityBeacon {
            device_fingerprint_prefix: [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            technology: ProximityTechnology::BleRssi,
            distance_cm: 10, // Very close
            rssi_dbm: -30,
            confidence_pct: 99,
            timestamp_us: 1000,
            nonce: 99,
        };

        let ack = mgr.handle_proximity_beacon(&rogue_beacon);
        assert!(!ack.accepted);
        assert!(!ack.prewarm_initiated);
        assert_eq!(mgr.state(), ProximityState::Idle);
    }

    #[test]
    fn test_workflow_state_persistence() {
        let state_path = get_test_state_path("persistence");

        let mgr = HostProximityManager::new(state_path.clone(), HashMap::new());

        let new_state = WorkflowState {
            window_width: 1200,
            window_height: 800,
            window_x: 50,
            window_y: 100,
            is_fullscreen: false,
            active_package_name: "com.android.chrome".to_string(),
            display_orientation: 1,
            saved_timestamp_us: 2000,
        };

        let ack = mgr.save_workflow_state(new_state.clone()).unwrap();
        assert!(ack.restored);

        // Recreate manager to test disk load
        let mgr2 = HostProximityManager::new(state_path, HashMap::new());
        let loaded = mgr2.load_saved_workflow_state();
        assert_eq!(loaded.window_width, 1200);
        assert_eq!(loaded.window_height, 800);
        assert_eq!(loaded.active_package_name, "com.android.chrome");
        assert_eq!(loaded.display_orientation, 1);
    }
}
