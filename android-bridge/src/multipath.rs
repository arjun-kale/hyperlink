//! Multipath scheduler and network resilience manager for Android companion.
//!
//! Monitors local network interfaces (Wi-Fi, USB/Cellular tether), sends and responds
//! to path probes (`0x90`/`0x91`), evaluates link RTT and loss, and orchestrates
//! zero-re-pairing failover when the primary route drops.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tracing::{info, warn};

use hyperlink_protocol::clock;
use hyperlink_protocol::resilience::{
    FailoverReason, PathHealthReport, PathKind, PathProbeRequest, PathProbeResponse, PathQuality,
    PathStatus, PathSwitchAck, PathSwitchNotice, PATH_ID_CELLULAR_TETHER, PATH_ID_ETHERNET,
    PATH_ID_PRIMARY_WIFI, PATH_ID_WIFI_MLO_SECONDARY,
};

/// Tracks active interfaces and health state.
#[derive(Debug)]
pub struct AndroidMultipathManager {
    /// Active transmission path ID.
    pub active_path_id: u8,
    /// Known network paths by path_id.
    pub paths: HashMap<u8, PathQuality>,
    /// Interface addresses mapped by path_id.
    pub interface_addrs: HashMap<u8, SocketAddr>,
    /// Session epoch counter (incremented per failover).
    pub session_epoch: u64,
    /// Next switch transaction ID.
    next_switch_id: u32,
    /// Last successful probe round-trip timestamp (local monotonic).
    last_probe_time: Option<Instant>,
}

impl Default for AndroidMultipathManager {
    fn default() -> Self {
        let mut paths = HashMap::new();
        let mut primary = PathQuality::new(PATH_ID_PRIMARY_WIFI, PathKind::WifiPrimary);
        primary.status = PathStatus::Active;
        paths.insert(PATH_ID_PRIMARY_WIFI, primary);

        let tether = PathQuality::new(PATH_ID_CELLULAR_TETHER, PathKind::CellularTether);
        paths.insert(PATH_ID_CELLULAR_TETHER, tether);

        let mlo = PathQuality::new(PATH_ID_WIFI_MLO_SECONDARY, PathKind::WifiMloSecondary);
        paths.insert(PATH_ID_WIFI_MLO_SECONDARY, mlo);

        let eth = PathQuality::new(PATH_ID_ETHERNET, PathKind::Ethernet);
        paths.insert(PATH_ID_ETHERNET, eth);

        Self {
            active_path_id: PATH_ID_PRIMARY_WIFI,
            paths,
            interface_addrs: HashMap::new(),
            session_epoch: 1,
            next_switch_id: 1,
            last_probe_time: None,
        }
    }
}

impl AndroidMultipathManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates or registers a network interface.
    pub fn register_interface(&mut self, path_id: u8, addr: SocketAddr) {
        self.interface_addrs.insert(path_id, addr);
        if let Some(path) = self.paths.get_mut(&path_id) {
            path.last_seen_us = clock::now_us();
            if path.status == PathStatus::Failed || path.status == PathStatus::Probing {
                path.status = if path_id == self.active_path_id {
                    PathStatus::Active
                } else {
                    PathStatus::Standby
                };
            }
        }
        info!(path_id = path_id, addr = %addr, "registered multipath network interface");
    }

    /// Marks an interface as lost / disconnected.
    pub fn mark_interface_lost(&mut self, path_id: u8) {
        self.interface_addrs.remove(&path_id);
        if let Some(path) = self.paths.get_mut(&path_id) {
            path.status = PathStatus::Failed;
        }
        warn!(path_id = path_id, "network interface marked lost");
    }

    /// Handles an incoming path probe request and creates a response.
    pub fn handle_probe_request(&mut self, req: &PathProbeRequest) -> PathProbeResponse {
        let recv_timestamp_us = clock::now_us();
        if let Some(path) = self.paths.get_mut(&req.path_id) {
            path.last_seen_us = recv_timestamp_us;
        }
        PathProbeResponse {
            req_id: req.req_id,
            path_id: req.path_id,
            send_timestamp_us: req.send_timestamp_us,
            recv_timestamp_us,
        }
    }

    /// Processes an incoming probe response to calculate path RTT and jitter.
    pub fn process_probe_response(&mut self, resp: &PathProbeResponse) {
        let now = clock::now_us();
        if now >= resp.send_timestamp_us {
            let rtt = (now - resp.send_timestamp_us) as u32;
            if let Some(path) = self.paths.get_mut(&resp.path_id) {
                let prev_rtt = path.rtt_us;
                path.rtt_us = rtt;
                if prev_rtt > 0 {
                    let diff = rtt.abs_diff(prev_rtt);
                    path.jitter_us = (path.jitter_us * 3 + diff) / 4;
                }
                path.last_seen_us = now;
                if path.status == PathStatus::Probing || path.status == PathStatus::Failed {
                    path.status = if resp.path_id == self.active_path_id {
                        PathStatus::Active
                    } else {
                        PathStatus::Standby
                    };
                }
            }
            self.last_probe_time = Some(Instant::now());
        }
    }

    /// Generates a PathSwitchNotice to migrate the connection to a standby path.
    pub fn initiate_failover(
        &mut self,
        target_path_id: u8,
        reason: FailoverReason,
    ) -> Option<PathSwitchNotice> {
        if target_path_id == self.active_path_id {
            return None;
        }

        let switch_id = self.next_switch_id;
        self.next_switch_id += 1;
        self.session_epoch += 1;

        let from_path = self.active_path_id;
        self.active_path_id = target_path_id;

        // Update states
        if let Some(old) = self.paths.get_mut(&from_path) {
            if old.status == PathStatus::Active {
                old.status = PathStatus::Degraded;
            }
        }
        if let Some(new_p) = self.paths.get_mut(&target_path_id) {
            new_p.status = PathStatus::Active;
        }

        info!(
            switch_id = switch_id,
            from_path = from_path,
            to_path = target_path_id,
            reason = ?reason,
            epoch = self.session_epoch,
            "initiated multipath failover"
        );

        Some(PathSwitchNotice {
            switch_id,
            from_path,
            to_path: target_path_id,
            reason,
            timestamp_us: clock::now_us(),
            session_epoch: self.session_epoch,
        })
    }

    /// Handles a PathSwitchNotice received from the host peer.
    pub fn handle_switch_notice(&mut self, notice: &PathSwitchNotice) -> PathSwitchAck {
        let from_path = notice.from_path;
        let to_path = notice.to_path;

        self.active_path_id = to_path;
        self.session_epoch = notice.session_epoch;

        if let Some(old) = self.paths.get_mut(&from_path) {
            if old.status == PathStatus::Active {
                old.status = PathStatus::Standby;
            }
        }
        if let Some(new_p) = self.paths.get_mut(&to_path) {
            new_p.status = PathStatus::Active;
        }

        info!(
            switch_id = notice.switch_id,
            from_path = from_path,
            to_path = to_path,
            reason = ?notice.reason,
            "accepted peer path switch notice"
        );

        PathSwitchAck {
            switch_id: notice.switch_id,
            accepted: true,
            ack_timestamp_us: clock::now_us(),
        }
    }

    /// Selects the best standby path when primary fails.
    pub fn select_best_standby(&self) -> Option<u8> {
        let mut candidates: Vec<(&u8, &PathQuality)> = self
            .paths
            .iter()
            .filter(|(&id, q)| {
                id != self.active_path_id
                    && (q.status == PathStatus::Standby || q.status == PathStatus::Probing)
            })
            .collect();

        // Sort by RTT ascending (prefer lowest RTT)
        candidates.sort_by_key(|(_, q)| if q.rtt_us > 0 { q.rtt_us } else { u32::MAX });

        candidates.first().map(|(&id, _)| id)
    }

    /// Returns a health report summary for all paths.
    pub fn generate_health_reports(&self) -> Vec<PathHealthReport> {
        self.paths
            .iter()
            .map(|(&id, q)| PathHealthReport {
                path_id: id,
                rtt_us: q.rtt_us,
                loss_percent_x100: q.loss_percent_x100,
                jitter_us: q.jitter_us,
                active: id == self.active_path_id,
            })
            .collect()
    }
}

/// Thread-safe global wrapper for Android multipath manager.
#[derive(Clone, Debug, Default)]
pub struct SharedMultipathManager(pub Arc<Mutex<AndroidMultipathManager>>);

impl SharedMultipathManager {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(AndroidMultipathManager::new())))
    }
}
