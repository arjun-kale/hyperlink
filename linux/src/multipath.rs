//! Multipath scheduling, path probing, and network resilience for Linux host (Phase 7).
//!
//! Manages primary (Wi-Fi 5GHz/6GHz) and secondary (Cellular Tether / Ethernet) paths,
//! periodically assesses link health and jitter via lightweight probes (`0x90`/`0x91`),
//! and performs zero-re-pairing failover when the primary route drops.

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

/// Host multipath manager state.
#[derive(Debug)]
pub struct HostMultipathManager {
    /// Active transmission path ID.
    pub active_path_id: u8,
    /// Known network paths by path_id.
    pub paths: HashMap<u8, PathQuality>,
    /// Interface socket addresses mapped by path_id.
    pub interface_addrs: HashMap<u8, SocketAddr>,
    /// Session epoch counter (incremented per failover).
    pub session_epoch: u64,
    /// Next switch transaction ID.
    next_switch_id: u32,
    /// Next probe request sequence ID.
    next_probe_id: u32,
    /// Last successful probe timestamp.
    pub last_probe_time: Option<Instant>,
    /// Whether a failover transition is currently in flight.
    pub failover_in_progress: bool,
}

impl Default for HostMultipathManager {
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
            next_probe_id: 1,
            last_probe_time: None,
            failover_in_progress: false,
        }
    }
}

impl HostMultipathManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or updates a path endpoint.
    pub fn register_path(&mut self, path_id: u8, addr: SocketAddr) {
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
        info!(path_id = path_id, addr = %addr, "host registered multipath route");
    }

    /// Creates a new probe request for a candidate path.
    pub fn create_probe_request(&mut self, path_id: u8) -> PathProbeRequest {
        let req_id = self.next_probe_id;
        self.next_probe_id += 1;
        PathProbeRequest {
            req_id,
            path_id,
            send_timestamp_us: clock::now_us(),
        }
    }

    /// Handles an incoming probe request from the companion.
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

    /// Processes a probe response to calculate path RTT and jitter.
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

    /// Alias for process_probe_response.
    pub fn handle_probe_response(&mut self, resp: &PathProbeResponse) {
        self.process_probe_response(resp);
    }

    /// Handles incoming link health report from peer.
    pub fn handle_health_report(&mut self, report: &PathHealthReport) {
        if let Some(path) = self.paths.get_mut(&report.path_id) {
            path.rtt_us = report.rtt_us;
            path.loss_percent_x100 = report.loss_percent_x100;
            path.last_seen_us = clock::now_us();
        }
    }

    /// Evaluates whether the current active path has degraded enough to warrant failover.
    pub fn should_failover(&self) -> Option<(u8, FailoverReason)> {
        if self.failover_in_progress {
            return None;
        }

        let active_quality = self.paths.get(&self.active_path_id)?;

        // If active path has failed or has high loss / latency
        let needs_switch = active_quality.status == PathStatus::Failed
            || active_quality.rtt_us > 150_000
            || active_quality.loss_percent_x100 > 2000; // > 20% loss

        if needs_switch {
            let reason = if active_quality.status == PathStatus::Failed {
                FailoverReason::LinkLoss
            } else if active_quality.loss_percent_x100 > 2000 {
                FailoverReason::HighLossRate
            } else {
                FailoverReason::HighJitter
            };

            // Find best standby candidate
            if let Some(target) = self.select_best_standby() {
                return Some((target, reason));
            }
        }

        None
    }

    /// Selects the best standby path.
    pub fn select_best_standby(&self) -> Option<u8> {
        let mut candidates: Vec<(&u8, &PathQuality)> = self
            .paths
            .iter()
            .filter(|(&id, q)| {
                id != self.active_path_id
                    && (q.status == PathStatus::Standby || q.status == PathStatus::Probing)
            })
            .collect();

        candidates.sort_by_key(|(_, q)| if q.rtt_us > 0 { q.rtt_us } else { u32::MAX });

        candidates.first().map(|(&id, _)| id)
    }

    /// Initiates a failover switch notice to the companion peer.
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
        self.failover_in_progress = true;

        let from_path = self.active_path_id;
        self.active_path_id = target_path_id;

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
            "host initiated multipath failover"
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

    /// Handles a PathSwitchAck from peer.
    pub fn handle_switch_ack(&mut self, ack: &PathSwitchAck) {
        if ack.accepted {
            self.failover_in_progress = false;
            info!(
                switch_id = ack.switch_id,
                "peer confirmed path switch completed"
            );
        } else {
            warn!(
                switch_id = ack.switch_id,
                "peer rejected path switch notice"
            );
        }
    }

    /// Handles an incoming PathSwitchNotice from companion peer.
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
            "host accepted companion path switch notice"
        );

        PathSwitchAck {
            switch_id: notice.switch_id,
            accepted: true,
            ack_timestamp_us: clock::now_us(),
        }
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

/// Shared reference for host multipath manager.
pub type SharedHostMultipathManager = Arc<Mutex<HostMultipathManager>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_multipath_lifecycle() {
        let mut manager = HostMultipathManager::new();
        assert_eq!(manager.active_path_id, PATH_ID_PRIMARY_WIFI);

        let tether_addr: SocketAddr = "192.168.43.1:9900".parse().unwrap();
        manager.register_path(PATH_ID_CELLULAR_TETHER, tether_addr);

        let tether = manager.paths.get(&PATH_ID_CELLULAR_TETHER).unwrap();
        assert_eq!(tether.status, PathStatus::Standby);

        // Probing
        let probe = manager.create_probe_request(PATH_ID_CELLULAR_TETHER);
        assert_eq!(probe.path_id, PATH_ID_CELLULAR_TETHER);

        let resp = PathProbeResponse {
            req_id: probe.req_id,
            path_id: PATH_ID_CELLULAR_TETHER,
            send_timestamp_us: probe.send_timestamp_us,
            recv_timestamp_us: probe.send_timestamp_us + 15_000,
        };
        manager.process_probe_response(&resp);

        // Failover initiation
        let notice = manager
            .initiate_failover(PATH_ID_CELLULAR_TETHER, FailoverReason::LinkLoss)
            .unwrap();
        assert_eq!(notice.from_path, PATH_ID_PRIMARY_WIFI);
        assert_eq!(notice.to_path, PATH_ID_CELLULAR_TETHER);
        assert_eq!(manager.active_path_id, PATH_ID_CELLULAR_TETHER);
        assert!(manager.failover_in_progress);

        // Ack
        let ack = PathSwitchAck {
            switch_id: notice.switch_id,
            accepted: true,
            ack_timestamp_us: clock::now_us(),
        };
        manager.handle_switch_ack(&ack);
        assert!(!manager.failover_in_progress);
    }
}
