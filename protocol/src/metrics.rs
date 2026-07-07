//! Measurement and reporting types for the bench harness.
//!
//! All metrics are serializable to JSON for structured logging — a cross-cutting
//! requirement from Phase 0 onward. "It feels fast" is not an acceptable DoD.

use serde::{Deserialize, Serialize};

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
    /// Target metrics for comparison.
    pub targets: TargetMetrics,
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
            targets: TargetMetrics::default(),
        };
        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("protocol_version"));
        assert!(json.contains("\"transport\": \"udp\""));
    }
}
