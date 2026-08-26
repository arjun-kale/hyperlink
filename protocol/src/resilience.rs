//! Network resilience and multipath wire protocol for Phase 7.
//!
//! Defines messages and types for multi-path probing, link health monitoring,
//! proactive MLO bonding / failover scheduling, and zero-re-pairing session migration
//! across Wi-Fi, cellular tethering, and Ethernet paths.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{self, Cursor};

/// Standard Path IDs for well-known transport routes.
pub const PATH_ID_PRIMARY_WIFI: u8 = 0;
pub const PATH_ID_CELLULAR_TETHER: u8 = 1;
pub const PATH_ID_ETHERNET: u8 = 2;
pub const PATH_ID_WIFI_MLO_SECONDARY: u8 = 3;

/// Path kind classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum PathKind {
    WifiPrimary = 0,
    CellularTether = 1,
    Ethernet = 2,
    WifiMloSecondary = 3,
    Custom = 255,
}

impl From<u8> for PathKind {
    fn from(val: u8) -> Self {
        match val {
            0 => Self::WifiPrimary,
            1 => Self::CellularTether,
            2 => Self::Ethernet,
            3 => Self::WifiMloSecondary,
            _ => Self::Custom,
        }
    }
}

/// Operational state of a network path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathStatus {
    /// Active primary transmission path.
    Active,
    /// Standby backup path actively probed for immediate failover.
    Standby,
    /// Probing / candidate path being evaluated.
    Probing,
    /// Degraded path with elevated jitter / packet loss.
    Degraded,
    /// Dead / unreachable path.
    Failed,
}

/// Reason code for a path switch / failover notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum FailoverReason {
    ManualSwitch = 0,
    LinkLoss = 1,
    HighJitter = 2,
    HighLossRate = 3,
    ProactiveMloSwitch = 4,
    KeepaliveTimeout = 5,
    Unknown = 255,
}

impl From<u8> for FailoverReason {
    fn from(val: u8) -> Self {
        match val {
            0 => Self::ManualSwitch,
            1 => Self::LinkLoss,
            2 => Self::HighJitter,
            3 => Self::HighLossRate,
            4 => Self::ProactiveMloSwitch,
            5 => Self::KeepaliveTimeout,
            _ => Self::Unknown,
        }
    }
}

/// Real-time quality metrics for a path candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathQuality {
    pub path_id: u8,
    pub path_kind: PathKind,
    pub rtt_us: u32,
    pub jitter_us: u32,
    pub loss_percent_x100: u16, // 0..=10000 (0.00% to 100.00%)
    pub bandwidth_kbps: u32,
    pub last_seen_us: u64,
    pub status: PathStatus,
}

impl PathQuality {
    pub fn new(path_id: u8, path_kind: PathKind) -> Self {
        Self {
            path_id,
            path_kind,
            rtt_us: 0,
            jitter_us: 0,
            loss_percent_x100: 0,
            bandwidth_kbps: 0,
            last_seen_us: 0,
            status: PathStatus::Probing,
        }
    }
}

/// Probe request for latency and health verification on a specific path (`0x90`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathProbeRequest {
    pub req_id: u32,
    pub path_id: u8,
    pub send_timestamp_us: u64,
}

impl PathProbeRequest {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u32::<BigEndian>(self.req_id)?;
        dst.write_u8(self.path_id)?;
        dst.write_u64::<BigEndian>(self.send_timestamp_us)?;
        Ok(())
    }

    pub fn decode(src: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(src);
        let req_id = cursor.read_u32::<BigEndian>()?;
        let path_id = cursor.read_u8()?;
        let send_timestamp_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            req_id,
            path_id,
            send_timestamp_us,
        })
    }
}

/// Probe response carrying return timestamp for path RTT calculation (`0x91`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathProbeResponse {
    pub req_id: u32,
    pub path_id: u8,
    pub send_timestamp_us: u64,
    pub recv_timestamp_us: u64,
}

impl PathProbeResponse {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u32::<BigEndian>(self.req_id)?;
        dst.write_u8(self.path_id)?;
        dst.write_u64::<BigEndian>(self.send_timestamp_us)?;
        dst.write_u64::<BigEndian>(self.recv_timestamp_us)?;
        Ok(())
    }

    pub fn decode(src: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(src);
        let req_id = cursor.read_u32::<BigEndian>()?;
        let path_id = cursor.read_u8()?;
        let send_timestamp_us = cursor.read_u64::<BigEndian>()?;
        let recv_timestamp_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            req_id,
            path_id,
            send_timestamp_us,
            recv_timestamp_us,
        })
    }
}

/// Explicit notice informing the peer of a path switch / failover transition (`0x92`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSwitchNotice {
    pub switch_id: u32,
    pub from_path: u8,
    pub to_path: u8,
    pub reason: FailoverReason,
    pub timestamp_us: u64,
    pub session_epoch: u64,
}

impl PathSwitchNotice {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u32::<BigEndian>(self.switch_id)?;
        dst.write_u8(self.from_path)?;
        dst.write_u8(self.to_path)?;
        dst.write_u8(self.reason as u8)?;
        dst.write_u64::<BigEndian>(self.timestamp_us)?;
        dst.write_u64::<BigEndian>(self.session_epoch)?;
        Ok(())
    }

    pub fn decode(src: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(src);
        let switch_id = cursor.read_u32::<BigEndian>()?;
        let from_path = cursor.read_u8()?;
        let to_path = cursor.read_u8()?;
        let reason_byte = cursor.read_u8()?;
        let timestamp_us = cursor.read_u64::<BigEndian>()?;
        let session_epoch = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            switch_id,
            from_path,
            to_path,
            reason: FailoverReason::from(reason_byte),
            timestamp_us,
            session_epoch,
        })
    }
}

/// Acknowledgement of path switch notice (`0x93`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSwitchAck {
    pub switch_id: u32,
    pub accepted: bool,
    pub ack_timestamp_us: u64,
}

impl PathSwitchAck {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u32::<BigEndian>(self.switch_id)?;
        dst.write_u8(if self.accepted { 1 } else { 0 })?;
        dst.write_u64::<BigEndian>(self.ack_timestamp_us)?;
        Ok(())
    }

    pub fn decode(src: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(src);
        let switch_id = cursor.read_u32::<BigEndian>()?;
        let accepted = cursor.read_u8()? != 0;
        let ack_timestamp_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            switch_id,
            accepted,
            ack_timestamp_us,
        })
    }
}

/// Periodic keepalive heartbeat on a candidate path (`0x94`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathHeartbeat {
    pub path_id: u8,
    pub seq: u32,
    pub timestamp_us: u64,
}

impl PathHeartbeat {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u8(self.path_id)?;
        dst.write_u32::<BigEndian>(self.seq)?;
        dst.write_u64::<BigEndian>(self.timestamp_us)?;
        Ok(())
    }

    pub fn decode(src: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(src);
        let path_id = cursor.read_u8()?;
        let seq = cursor.read_u32::<BigEndian>()?;
        let timestamp_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            path_id,
            seq,
            timestamp_us,
        })
    }
}

/// Periodic health report summarizing link condition (`0x95`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathHealthReport {
    pub path_id: u8,
    pub rtt_us: u32,
    pub loss_percent_x100: u16,
    pub jitter_us: u32,
    pub active: bool,
}

impl PathHealthReport {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u8(self.path_id)?;
        dst.write_u32::<BigEndian>(self.rtt_us)?;
        dst.write_u16::<BigEndian>(self.loss_percent_x100)?;
        dst.write_u32::<BigEndian>(self.jitter_us)?;
        dst.write_u8(if self.active { 1 } else { 0 })?;
        Ok(())
    }

    pub fn decode(src: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(src);
        let path_id = cursor.read_u8()?;
        let rtt_us = cursor.read_u32::<BigEndian>()?;
        let loss_percent_x100 = cursor.read_u16::<BigEndian>()?;
        let jitter_us = cursor.read_u32::<BigEndian>()?;
        let active = cursor.read_u8()? != 0;
        Ok(Self {
            path_id,
            rtt_us,
            loss_percent_x100,
            jitter_us,
            active,
        })
    }
}

/// Statistics from a network resilience / multipath failover benchmark session (Phase 7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResilienceBenchStats {
    /// Total failover scenarios executed.
    pub total_failovers_tested: u64,
    /// Total successful failovers without disconnection or re-pairing.
    pub successful_failovers: u64,
    /// Average time to detect primary path fault in milliseconds.
    pub failover_detection_time_ms: f64,
    /// Average time to switch transmission path in milliseconds.
    pub failover_switch_time_ms: f64,
    /// Total end-to-end failover latency (detection + switch) in milliseconds.
    pub total_failover_latency_ms: f64,
    /// Number of video frames sent before fault injection.
    pub video_frames_before_fault: u64,
    /// Number of video frames transmitted during failover transition.
    pub video_frames_during_failover: u64,
    /// Number of video frames received after recovery on secondary path.
    pub video_frames_after_recovery: u64,
    /// Whether video datagram stream survived failover without pipeline restart.
    pub video_stream_survived: bool,
    /// Total input events dispatched across session.
    pub input_events_sent: u64,
    /// Total input events delivered across primary + secondary paths.
    pub input_events_delivered: u64,
    /// Total input events lost during failover.
    pub input_events_lost: u64,
    /// Whether input reliable stream survived with 0 loss.
    pub input_stream_survived: bool,
    /// Whether zero re-pairing was confirmed (session tokens & crypto keys persisted).
    pub zero_repairing_verified: bool,
    /// Whether all Phase 7 DoD targets were met.
    pub target_met: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_probe_round_trip() {
        let req = PathProbeRequest {
            req_id: 42,
            path_id: PATH_ID_PRIMARY_WIFI,
            send_timestamp_us: 1_700_000_000_123,
        };
        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();
        let decoded = PathProbeRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);

        let resp = PathProbeResponse {
            req_id: 42,
            path_id: PATH_ID_PRIMARY_WIFI,
            send_timestamp_us: 1_700_000_000_123,
            recv_timestamp_us: 1_700_000_000_456,
        };
        let mut resp_buf = Vec::new();
        resp.encode(&mut resp_buf).unwrap();
        let decoded_resp = PathProbeResponse::decode(&resp_buf).unwrap();
        assert_eq!(resp, decoded_resp);
    }

    #[test]
    fn test_path_switch_round_trip() {
        let notice = PathSwitchNotice {
            switch_id: 1001,
            from_path: PATH_ID_PRIMARY_WIFI,
            to_path: PATH_ID_CELLULAR_TETHER,
            reason: FailoverReason::LinkLoss,
            timestamp_us: 1_700_000_000_789,
            session_epoch: 12,
        };
        let mut buf = Vec::new();
        notice.encode(&mut buf).unwrap();
        let decoded = PathSwitchNotice::decode(&buf).unwrap();
        assert_eq!(notice, decoded);

        let ack = PathSwitchAck {
            switch_id: 1001,
            accepted: true,
            ack_timestamp_us: 1_700_000_000_890,
        };
        let mut ack_buf = Vec::new();
        ack.encode(&mut ack_buf).unwrap();
        let decoded_ack = PathSwitchAck::decode(&ack_buf).unwrap();
        assert_eq!(ack, decoded_ack);
    }

    #[test]
    fn test_path_heartbeat_and_report_round_trip() {
        let hb = PathHeartbeat {
            path_id: PATH_ID_CELLULAR_TETHER,
            seq: 55,
            timestamp_us: 123456789,
        };
        let mut hb_buf = Vec::new();
        hb.encode(&mut hb_buf).unwrap();
        let decoded_hb = PathHeartbeat::decode(&hb_buf).unwrap();
        assert_eq!(hb, decoded_hb);

        let report = PathHealthReport {
            path_id: PATH_ID_PRIMARY_WIFI,
            rtt_us: 15200,
            loss_percent_x100: 50, // 0.50%
            jitter_us: 1800,
            active: true,
        };
        let mut report_buf = Vec::new();
        report.encode(&mut report_buf).unwrap();
        let decoded_report = PathHealthReport::decode(&report_buf).unwrap();
        assert_eq!(report, decoded_report);
    }
}
