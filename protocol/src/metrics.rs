//! Measurement and reporting types for the bench harness.
//!
//! All metrics are serializable to JSON for structured logging — a cross-cutting
//! requirement from Phase 0 onward. "It feels fast" is not an acceptable DoD.

use serde::{Deserialize, Serialize};

pub use crate::resilience::ResilienceBenchStats;

/// A single latency measurement from one echo round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyMeasurement {
    /// Sequence number of this measurement.
    pub seq: u32,
    /// Round-trip time in microseconds.
    pub rtt_us: i64,
    /// Estimated one-way latency in microseconds (RTT / 2, unless clock sync
    /// provides a better estimate).
    pub estimated_one_way_us: i64,
    /// Timestamp of this measurement (ISO 8601).
    pub timestamp: String,
}

/// Result of a clock synchronization session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockSyncResult {
    /// Number of sync rounds performed.
    pub rounds: u32,
    /// Best (minimum) estimated clock offset in microseconds.
    /// Positive means server clock is ahead.
    pub offset_us: i64,
    /// Best (minimum) round-trip delay in microseconds.
    pub min_rtt_us: i64,
    /// Mean round-trip delay across all rounds.
    pub mean_rtt_us: i64,
    /// Estimated drift in microseconds per second over the measurement window.
    pub drift_us_per_sec: f64,
    /// Duration of the drift measurement window in seconds.
    pub window_secs: f64,
    /// Whether drift is within the 1ms/60s DoD threshold.
    pub drift_within_spec: bool,
}

/// Aggregated statistics from a batch of latency measurements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
    /// Number of measurements.
    pub count: u64,
    /// Minimum RTT in microseconds.
    pub min_us: i64,
    /// Maximum RTT in microseconds.
    pub max_us: i64,
    /// Mean RTT in microseconds.
    pub mean_us: f64,
    /// Median (p50) RTT in microseconds.
    pub p50_us: i64,
    /// 95th percentile RTT in microseconds.
    pub p95_us: i64,
    /// 99th percentile RTT in microseconds.
    pub p99_us: i64,
    /// Standard deviation in microseconds.
    pub stddev_us: f64,
    /// Number of lost/timed-out packets.
    pub lost: u64,
}

/// Target metrics table — values are filled in once real hardware measurements
/// exist (Phase 2+). Until then, these are suggested starting targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetMetrics {
    /// Video glass-to-glass latency targets by transport.
    pub video_latency: TransportTargets,
    /// Input round-trip latency targets by transport.
    pub input_rtt: TransportTargets,
    /// Notification propagation latency targets by transport.
    pub notification_latency: TransportTargets,
    /// Clipboard sync latency targets by transport.
    pub clipboard_latency: TransportTargets,
    /// File throughput targets in MB/s by transport.
    pub file_throughput_mbps: TransportTargets,
    /// File time-to-first-byte targets by transport.
    pub file_ttfb: TransportTargets,
}

/// Per-transport target values (microseconds for latency, MB/s for throughput).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportTargets {
    /// USB target value.
    pub usb: Option<f64>,
    /// 5 GHz WiFi target value.
    pub wifi_5ghz: Option<f64>,
    /// 6 GHz WiFi target value.
    pub wifi_6ghz: Option<f64>,
}

impl TransportTargets {
    /// Create targets with no values set (TBD).
    pub fn tbd() -> Self {
        Self {
            usb: None,
            wifi_5ghz: None,
            wifi_6ghz: None,
        }
    }
}

impl Default for TargetMetrics {
    /// Default target metrics with the suggested starting values from the design doc.
    fn default() -> Self {
        Self {
            video_latency: TransportTargets {
                usb: Some(60_000.0),        // <60ms = 60,000µs
                wifi_5ghz: Some(100_000.0), // <100ms = 100,000µs
                wifi_6ghz: None,            // TBD
            },
            input_rtt: TransportTargets::tbd(),
            notification_latency: TransportTargets::tbd(),
            clipboard_latency: TransportTargets::tbd(),
            file_throughput_mbps: TransportTargets::tbd(),
            file_ttfb: TransportTargets::tbd(),
        }
    }
}

/// Statistics from a video streaming benchmark session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoBenchStats {
    /// Total video frames sent.
    pub total_frames_sent: u64,
    /// Total video frames successfully received and assembled.
    pub total_frames_received: u64,
    /// Total frames dropped (incomplete fragments or stale).
    pub total_frames_dropped: u64,
    /// Frame drop rate percentage (0.0 - 100.0).
    pub frame_drop_rate_pct: f64,
    /// Latency statistics across received frames (in microseconds).
    pub latency_stats: LatencyStats,
    /// Measured effective frames per second.
    pub achieved_fps: f64,
    /// Measured average video bitrate in kbps.
    pub average_bitrate_kbps: f64,
    /// Simulated packet loss rate percentage, if injected.
    pub loss_simulated_pct: Option<f64>,
    /// Whether measured latency satisfies the target threshold (<60ms USB, <100ms WiFi).
    pub target_met: bool,
}

/// Statistics from an input latency benchmark session (Phase 3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputBenchStats {
    /// Total input events dispatched.
    pub total_events: u64,
    /// Total acknowledgements received.
    pub total_acks: u64,
    /// Total unacknowledged/lost input events.
    pub lost_events: u64,
    /// Loss rate percentage.
    pub loss_rate_pct: f64,
    /// Round-trip latency statistics in microseconds.
    pub rtt_stats: LatencyStats,
    /// Whether measured p95 RTT satisfies input targets.
    pub target_met: bool,
}

/// Complete bench report for a measurement session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReport {
    /// Report timestamp (ISO 8601).
    pub timestamp: String,
    /// HyperLink protocol version used.
    pub protocol_version: u8,
    /// Transport type (e.g., "udp", "quic-usb", "quic-wifi-5ghz").
    pub transport: String,
    /// Clock synchronization results (if performed).
    pub clock_sync: Option<ClockSyncResult>,
    /// Echo latency statistics (if performed).
    pub echo_stats: Option<LatencyStats>,
    /// Individual echo measurements (for detailed analysis).
    pub echo_measurements: Vec<LatencyMeasurement>,
    /// Video stream benchmark statistics (if performed).
    #[serde(default)]
    pub video_stats: Option<VideoBenchStats>,
    /// Input latency benchmark statistics (if performed).
    #[serde(default)]
    pub input_stats: Option<InputBenchStats>,
    /// Notification sync benchmark statistics (if performed).
    #[serde(default)]
    pub notification_stats: Option<NotificationBenchStats>,
    /// Clipboard sync benchmark statistics (if performed).
    #[serde(default)]
    pub clipboard_stats: Option<ClipboardBenchStats>,
    /// File access benchmark statistics (if performed).
    #[serde(default)]
    pub file_stats: Option<FileBenchStats>,
    /// Network resilience / multipath benchmark statistics (if performed).
    #[serde(default)]
    pub resilience_stats: Option<crate::resilience::ResilienceBenchStats>,
    /// Proximity & pre-warmed connect benchmark statistics (if performed).
    #[serde(default)]
    pub proximity_stats: Option<ProximityBenchStats>,
    /// Scoped app-state handoff benchmark statistics (if performed).
    #[serde(default)]
    pub handoff_stats: Option<HandoffBenchStats>,
    /// Ambient context agent benchmark statistics (if performed).
    #[serde(default)]
    pub ambient_stats: Option<AmbientBenchStats>,
    /// Target metrics for comparison.
    pub targets: TargetMetrics,
}

/// Statistics from an ambient context agent benchmark session (Phase 10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmbientBenchStats {
    /// Total ambient events published and ingested.
    pub total_events_published: u64,
    /// Total ambient events successfully delivered to subscribers.
    pub total_events_delivered: u64,
    /// Mean event publishing / ingestion latency in microseconds.
    pub avg_publish_latency_us: f64,
    /// Median (p50) publish latency in microseconds.
    pub p50_latency_us: u64,
    /// 95th percentile (p95) publish latency in microseconds.
    pub p95_latency_us: u64,
    /// Whether consent policy gating was 100% verified (blocked categories strictly dropped).
    pub consent_gating_verified: bool,
    /// Security invariant check: verified that raw video frames are sandboxed from ambient agents.
    pub raw_video_sandboxed: bool,
    /// Whether agent timeline query synthesis produced 100% accurate citations and answers.
    pub agent_query_fidelity: bool,
    /// Whether Phase 10 DoD quality gate targets are met.
    pub target_met: bool,
}

/// Statistics from a scoped app-state handoff benchmark session (Phase 9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffBenchStats {
    /// Total state handoff events tested.
    pub total_handoffs_tested: u64,
    /// Total successfully dispatched & launched handoffs.
    pub successful_handoffs: u64,
    /// Average end-to-end handoff latency in milliseconds.
    pub avg_handoff_latency_ms: f64,
    /// Median (p50) handoff latency in microseconds.
    pub p50_latency_us: u64,
    /// 95th percentile (p95) handoff latency in microseconds.
    pub p95_latency_us: u64,
    /// 99th percentile (p99) handoff latency in microseconds.
    pub p99_latency_us: u64,
    /// Whether state payload was restored with 100% fidelity (cursor, scroll, position match).
    pub state_restoration_fidelity: bool,
    /// Whether the handoff JSON payload adheres strictly to the documented schema.
    pub contract_schema_verified: bool,
    /// Whether Phase 9 DoD quality gate targets are met.
    pub target_met: bool,
}

/// Statistics from a proximity & pre-warmed connect benchmark session (Phase 8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProximityBenchStats {
    /// Cold connection setup latency in milliseconds (discovery + mTLS handshake + stream ready).
    pub cold_setup_latency_ms: f64,
    /// Pre-warmed connection setup latency in milliseconds (instantaneous mirror activate).
    pub prewarmed_setup_latency_ms: f64,
    /// Setup latency reduction percentage (e.g. 78.5%).
    pub latency_reduction_pct: f64,
    /// Measured time from proximity in-range detection to usable mirror in milliseconds.
    pub time_from_in_range_to_mirror_ms: f64,
    /// Whether saved workflow state (window geometry, orientation, active app) was restored with 100% fidelity.
    pub workflow_state_restored: bool,
    /// Security invariant check: verified that proximity data alone never bypasses cert-based mTLS auth.
    pub security_auth_bypass_prevented: bool,
    /// Whether Phase 8 DoD quality gate targets are met.
    pub target_met: bool,
}

/// Statistics from a notification sync benchmark session (Phase 4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationBenchStats {
    /// Total notifications dispatched.
    pub total_posted: u64,
    /// Total notification acknowledgements received.
    pub total_acks: u64,
    /// Total lost / unacknowledged notifications.
    pub lost_count: u64,
    /// Notification delivery loss rate percentage.
    pub loss_rate_pct: f64,
    /// Propagation latency statistics in microseconds.
    pub propagation_latency_stats: LatencyStats,
    /// Total action button invocations tested.
    pub total_actions_invoked: u64,
    /// Total action invocation acks received.
    pub action_acks_received: u64,
    /// Action invocation round-trip latency statistics in microseconds.
    pub action_rtt_stats: Option<LatencyStats>,
    /// Whether measured propagation p95 and loss satisfy Phase 4 DoD targets.
    pub target_met: bool,
}

/// Statistics from a clipboard sync benchmark session (Phase 5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardBenchStats {
    /// Total clipboard syncs attempted.
    pub total_syncs: u64,
    /// Total clipboard acks received.
    pub total_acks: u64,
    /// Total text syncs.
    pub text_syncs: u64,
    /// Total image syncs.
    pub image_syncs: u64,
    /// Total bytes transferred.
    pub total_bytes: u64,
    /// Sync propagation latency statistics in microseconds.
    pub sync_latency_stats: LatencyStats,
    /// Loop prevention test: number of echo loops detected (must be 0).
    pub loop_echoes_detected: u64,
    /// Whether ping-pong loop prevention test passed (0 echoes).
    pub loop_prevention_passed: bool,
    /// Concurrent input stream p95 latency under bulk clipboard load (in µs).
    pub concurrent_input_p95_us: Option<i64>,
    /// Whether multiplexing DoD target was met (p95 <= 100ms, 0 loops, input not starved).
    pub target_met: bool,
}

/// Statistics from a file access benchmark session (Phase 6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileBenchStats {
    /// Total read operations.
    pub total_reads: u64,
    /// Total write operations.
    pub total_writes: u64,
    /// Total bytes read via chunked read.
    pub total_bytes_read: u64,
    /// Total bytes written via chunked write.
    pub total_bytes_written: u64,
    /// Time-to-first-byte (TTFB) latency statistics in microseconds.
    pub ttfb_stats: LatencyStats,
    /// Achieved read throughput in MB/s.
    pub throughput_mbps: f64,
    /// Whether lazy seeking (random scrub) avoided downloading prior bytes.
    pub lazy_seek_verified: bool,
    /// Whether write-back of modified chunks was verified.
    pub writeback_verified: bool,
    /// Whether checksum of fully read file matched source byte-for-byte.
    pub checksum_verified: bool,
    /// Whether all Phase 6 DoD targets were met.
    pub target_met: bool,
}

impl LatencyStats {
    /// Compute aggregate statistics from a slice of RTT measurements (in µs).
    ///
    /// The input slice must not be empty.
    pub fn from_rtts(rtts: &[i64], lost: u64) -> Self {
        assert!(!rtts.is_empty(), "cannot compute stats from empty slice");

        let mut sorted = rtts.to_vec();
        sorted.sort_unstable();

        let count = sorted.len() as u64;
        let min_us = sorted[0];
        let max_us = *sorted.last().unwrap();
        let sum: i64 = sorted.iter().sum();
        let mean_us = sum as f64 / count as f64;

        let p50_us = percentile(&sorted, 50.0);
        let p95_us = percentile(&sorted, 95.0);
        let p99_us = percentile(&sorted, 99.0);

        let variance: f64 = sorted
            .iter()
            .map(|&x| {
                let diff = x as f64 - mean_us;
                diff * diff
            })
            .sum::<f64>()
            / count as f64;
        let stddev_us = variance.sqrt();

        Self {
            count,
            min_us,
            max_us,
            mean_us,
            p50_us,
            p95_us,
            p99_us,
            stddev_us,
            lost,
        }
    }
}

/// Compute a percentile value from a sorted slice using nearest-rank method.
fn percentile(sorted: &[i64], pct: f64) -> i64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = (pct / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_basic() {
        let rtts = vec![100, 200, 300, 400, 500];
        let stats = LatencyStats::from_rtts(&rtts, 0);
        assert_eq!(stats.count, 5);
        assert_eq!(stats.min_us, 100);
        assert_eq!(stats.max_us, 500);
        assert!((stats.mean_us - 300.0).abs() < f64::EPSILON);
        assert_eq!(stats.lost, 0);
    }

    #[test]
    fn stats_single_value() {
        let rtts = vec![42];
        let stats = LatencyStats::from_rtts(&rtts, 0);
        assert_eq!(stats.count, 1);
        assert_eq!(stats.min_us, 42);
        assert_eq!(stats.max_us, 42);
        assert_eq!(stats.p50_us, 42);
        assert_eq!(stats.p95_us, 42);
        assert_eq!(stats.p99_us, 42);
    }

    #[test]
    fn target_metrics_default_has_video_targets() {
        let targets = TargetMetrics::default();
        assert_eq!(targets.video_latency.usb, Some(60_000.0));
        assert_eq!(targets.video_latency.wifi_5ghz, Some(100_000.0));
        assert!(targets.input_rtt.usb.is_none());
    }

    #[test]
    fn bench_report_serializes_to_json() {
        let report = BenchReport {
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            protocol_version: 1,
            transport: "udp".to_string(),
            clock_sync: None,
            echo_stats: None,
            echo_measurements: vec![],
            video_stats: None,
            input_stats: None,
            notification_stats: None,
            clipboard_stats: None,
            file_stats: None,
            resilience_stats: None,
            proximity_stats: None,
            handoff_stats: None,
            ambient_stats: None,
            targets: TargetMetrics::default(),
        };
        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("protocol_version"));
        assert!(json.contains("\"transport\": \"udp\""));
    }

    #[test]
    fn input_bench_stats_serialization() {
        let stats = InputBenchStats {
            total_events: 100,
            total_acks: 99,
            lost_events: 1,
            loss_rate_pct: 1.0,
            rtt_stats: LatencyStats {
                count: 99,
                min_us: 1000,
                max_us: 12000,
                mean_us: 4500.0,
                p50_us: 4000,
                p95_us: 8000,
                p99_us: 11000,
                stddev_us: 1500.0,
                lost: 1,
            },
            target_met: true,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"total_events\":100"));
        assert!(json.contains("\"target_met\":true"));
    }

    #[test]
    fn video_bench_stats_serialization() {
        let stats = VideoBenchStats {
            total_frames_sent: 300,
            total_frames_received: 295,
            total_frames_dropped: 5,
            frame_drop_rate_pct: 1.67,
            latency_stats: LatencyStats::from_rtts(&[12000, 15000, 18000], 5),
            achieved_fps: 29.8,
            average_bitrate_kbps: 4120.5,
            loss_simulated_pct: Some(2.0),
            target_met: true,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("achieved_fps"));
        assert!(json.contains("29.8"));
        assert!(json.contains("target_met"));
    }

    #[test]
    fn notification_bench_stats_serialization() {
        let stats = NotificationBenchStats {
            total_posted: 50,
            total_acks: 50,
            lost_count: 0,
            loss_rate_pct: 0.0,
            propagation_latency_stats: LatencyStats::from_rtts(&[15000, 22000, 31000], 0),
            total_actions_invoked: 10,
            action_acks_received: 10,
            action_rtt_stats: Some(LatencyStats::from_rtts(&[12000, 18000], 0)),
            target_met: true,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"total_posted\":50"));
        assert!(json.contains("\"target_met\":true"));
    }

    #[test]
    fn clipboard_bench_stats_serialization() {
        let stats = ClipboardBenchStats {
            total_syncs: 20,
            total_acks: 20,
            text_syncs: 15,
            image_syncs: 5,
            total_bytes: 1024 * 1024 * 5,
            sync_latency_stats: LatencyStats::from_rtts(&[5000, 10000, 25000], 0),
            loop_echoes_detected: 0,
            loop_prevention_passed: true,
            concurrent_input_p95_us: Some(15000),
            target_met: true,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"total_syncs\":20"));
        assert!(json.contains("\"loop_prevention_passed\":true"));
        assert!(json.contains("\"target_met\":true"));
    }

    #[test]
    fn file_bench_stats_serialization() {
        let stats = FileBenchStats {
            total_reads: 50,
            total_writes: 5,
            total_bytes_read: 50 * 64 * 1024,
            total_bytes_written: 5 * 64 * 1024,
            ttfb_stats: LatencyStats::from_rtts(&[1200, 2500, 4800], 0),
            throughput_mbps: 45.5,
            lazy_seek_verified: true,
            writeback_verified: true,
            checksum_verified: true,
            target_met: true,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"total_reads\":50"));
        assert!(json.contains("\"lazy_seek_verified\":true"));
        assert!(json.contains("\"target_met\":true"));
    }

    #[test]
    fn resilience_bench_stats_serialization() {
        let stats = crate::resilience::ResilienceBenchStats {
            total_failovers_tested: 5,
            successful_failovers: 5,
            failover_detection_time_ms: 42.5,
            failover_switch_time_ms: 18.2,
            total_failover_latency_ms: 60.7,
            video_frames_before_fault: 150,
            video_frames_during_failover: 12,
            video_frames_after_recovery: 138,
            video_stream_survived: true,
            input_events_sent: 50,
            input_events_delivered: 50,
            input_events_lost: 0,
            input_stream_survived: true,
            zero_repairing_verified: true,
            target_met: true,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"total_failovers_tested\":5"));
        assert!(json.contains("\"video_stream_survived\":true"));
        assert!(json.contains("\"target_met\":true"));
    }

    #[test]
    fn proximity_bench_stats_serialization() {
        let stats = ProximityBenchStats {
            cold_setup_latency_ms: 245.0,
            prewarmed_setup_latency_ms: 32.5,
            latency_reduction_pct: 86.7,
            time_from_in_range_to_mirror_ms: 65.0,
            workflow_state_restored: true,
            security_auth_bypass_prevented: true,
            target_met: true,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"cold_setup_latency_ms\":245.0"));
        assert!(json.contains("\"workflow_state_restored\":true"));
        assert!(json.contains("\"security_auth_bypass_prevented\":true"));
        assert!(json.contains("\"target_met\":true"));
    }

    #[test]
    fn handoff_bench_stats_serialization() {
        let stats = HandoffBenchStats {
            total_handoffs_tested: 10,
            successful_handoffs: 10,
            avg_handoff_latency_ms: 42.5,
            p50_latency_us: 38000,
            p95_latency_us: 65000,
            p99_latency_us: 82000,
            state_restoration_fidelity: true,
            contract_schema_verified: true,
            target_met: true,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"total_handoffs_tested\":10"));
        assert!(json.contains("\"avg_handoff_latency_ms\":42.5"));
        assert!(json.contains("\"state_restoration_fidelity\":true"));
        assert!(json.contains("\"target_met\":true"));
    }

    #[test]
    fn ambient_bench_stats_serialization() {
        let stats = AmbientBenchStats {
            total_events_published: 50,
            total_events_delivered: 45,
            avg_publish_latency_us: 1250.0,
            p50_latency_us: 1100,
            p95_latency_us: 2800,
            consent_gating_verified: true,
            raw_video_sandboxed: true,
            agent_query_fidelity: true,
            target_met: true,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"total_events_published\":50"));
        assert!(json.contains("\"consent_gating_verified\":true"));
        assert!(json.contains("\"raw_video_sandboxed\":true"));
        assert!(json.contains("\"target_met\":true"));
    }
}
