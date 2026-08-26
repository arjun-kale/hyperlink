//! Report formatting and file output.
//!
//! Outputs bench results as structured JSON (for machine consumption) and
//! a human-readable table (for terminal display). Each run appends a
//! timestamped JSON line to a results file for historical tracking.

use std::path::{Path, PathBuf};

use chrono::Utc;
use tracing::info;

use hyperlink_protocol::metrics::{
    BenchReport, ClipboardBenchStats, ClockSyncResult, FileBenchStats, InputBenchStats,
    LatencyMeasurement, LatencyStats, ResilienceBenchStats, TargetMetrics, VideoBenchStats,
};
use hyperlink_protocol::version::PROTOCOL_VERSION;

/// Build a complete `BenchReport` from the measurement results.
pub fn build_report(
    transport_name: &str,
    clock_sync: Option<ClockSyncResult>,
    echo_stats: Option<LatencyStats>,
    echo_measurements: Vec<LatencyMeasurement>,
) -> BenchReport {
    BenchReport {
        timestamp: Utc::now().to_rfc3339(),
        protocol_version: PROTOCOL_VERSION,
        transport: transport_name.to_string(),
        clock_sync,
        echo_stats,
        echo_measurements,
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
    }
}

/// Build a `BenchReport` specifically for video streaming benchmark.
pub fn build_video_report(transport_name: &str, video_stats: VideoBenchStats) -> BenchReport {
    BenchReport {
        timestamp: Utc::now().to_rfc3339(),
        protocol_version: PROTOCOL_VERSION,
        transport: transport_name.to_string(),
        clock_sync: None,
        echo_stats: None,
        echo_measurements: vec![],
        video_stats: Some(video_stats),
        input_stats: None,
        notification_stats: None,
        clipboard_stats: None,
        file_stats: None,
        resilience_stats: None,
        proximity_stats: None,
        handoff_stats: None,
        ambient_stats: None,
        targets: TargetMetrics::default(),
    }
}

/// Build a `BenchReport` specifically for input latency benchmark (Phase 3).
pub fn build_input_report(transport_name: &str, input_stats: InputBenchStats) -> BenchReport {
    BenchReport {
        timestamp: Utc::now().to_rfc3339(),
        protocol_version: PROTOCOL_VERSION,
        transport: transport_name.to_string(),
        clock_sync: None,
        echo_stats: None,
        echo_measurements: vec![],
        video_stats: None,
        input_stats: Some(input_stats),
        notification_stats: None,
        clipboard_stats: None,
        file_stats: None,
        resilience_stats: None,
        proximity_stats: None,
        handoff_stats: None,
        ambient_stats: None,
        targets: TargetMetrics::default(),
    }
}

/// Build a `BenchReport` specifically for notification sync benchmark (Phase 4).
pub fn build_notification_report(
    transport_name: &str,
    notification_stats: hyperlink_protocol::metrics::NotificationBenchStats,
) -> BenchReport {
    BenchReport {
        timestamp: Utc::now().to_rfc3339(),
        protocol_version: PROTOCOL_VERSION,
        transport: transport_name.to_string(),
        clock_sync: None,
        echo_stats: None,
        echo_measurements: vec![],
        video_stats: None,
        input_stats: None,
        notification_stats: Some(notification_stats),
        clipboard_stats: None,
        file_stats: None,
        resilience_stats: None,
        proximity_stats: None,
        handoff_stats: None,
        ambient_stats: None,
        targets: TargetMetrics::default(),
    }
}

/// Build a `BenchReport` specifically for clipboard sync benchmark (Phase 5).
pub fn build_clipboard_report(
    transport_name: &str,
    clipboard_stats: ClipboardBenchStats,
) -> BenchReport {
    BenchReport {
        timestamp: Utc::now().to_rfc3339(),
        protocol_version: PROTOCOL_VERSION,
        transport: transport_name.to_string(),
        clock_sync: None,
        echo_stats: None,
        echo_measurements: vec![],
        video_stats: None,
        input_stats: None,
        notification_stats: None,
        clipboard_stats: Some(clipboard_stats),
        file_stats: None,
        resilience_stats: None,
        proximity_stats: None,
        handoff_stats: None,
        ambient_stats: None,
        targets: TargetMetrics::default(),
    }
}

/// Build a `BenchReport` specifically for file access benchmark (Phase 6).
pub fn build_file_report(transport_name: &str, file_stats: FileBenchStats) -> BenchReport {
    BenchReport {
        timestamp: Utc::now().to_rfc3339(),
        protocol_version: PROTOCOL_VERSION,
        transport: transport_name.to_string(),
        clock_sync: None,
        echo_stats: None,
        echo_measurements: vec![],
        video_stats: None,
        input_stats: None,
        notification_stats: None,
        clipboard_stats: None,
        file_stats: Some(file_stats),
        resilience_stats: None,
        proximity_stats: None,
        handoff_stats: None,
        ambient_stats: None,
        targets: TargetMetrics::default(),
    }
}

/// Build a `BenchReport` specifically for network resilience benchmark (Phase 7).
pub fn build_resilience_report(
    transport_name: &str,
    resilience_stats: ResilienceBenchStats,
) -> BenchReport {
    BenchReport {
        timestamp: Utc::now().to_rfc3339(),
        protocol_version: PROTOCOL_VERSION,
        transport: transport_name.to_string(),
        clock_sync: None,
        echo_stats: None,
        echo_measurements: vec![],
        video_stats: None,
        input_stats: None,
        notification_stats: None,
        clipboard_stats: None,
        file_stats: None,
        resilience_stats: Some(resilience_stats),
        proximity_stats: None,
        handoff_stats: None,
        ambient_stats: None,
        targets: TargetMetrics::default(),
    }
}

/// Build a `BenchReport` specifically for proximity & pre-warmed connect benchmark (Phase 8).
pub fn build_proximity_report(
    transport_name: &str,
    proximity_stats: hyperlink_protocol::metrics::ProximityBenchStats,
) -> BenchReport {
    BenchReport {
        timestamp: Utc::now().to_rfc3339(),
        protocol_version: PROTOCOL_VERSION,
        transport: transport_name.to_string(),
        clock_sync: None,
        echo_stats: None,
        echo_measurements: vec![],
        video_stats: None,
        input_stats: None,
        notification_stats: None,
        clipboard_stats: None,
        file_stats: None,
        resilience_stats: None,
        proximity_stats: Some(proximity_stats),
        handoff_stats: None,
        ambient_stats: None,
        targets: TargetMetrics::default(),
    }
}

/// Build a `BenchReport` specifically for scoped app-state handoff benchmark (Phase 9).
pub fn build_handoff_report(
    transport_name: &str,
    handoff_stats: hyperlink_protocol::metrics::HandoffBenchStats,
) -> BenchReport {
    BenchReport {
        timestamp: Utc::now().to_rfc3339(),
        protocol_version: PROTOCOL_VERSION,
        transport: transport_name.to_string(),
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
        handoff_stats: Some(handoff_stats),
        ambient_stats: None,
        targets: TargetMetrics::default(),
    }
}

/// Build a `BenchReport` specifically for ambient context agent benchmark (Phase 10).
pub fn build_ambient_report(
    transport_name: &str,
    ambient_stats: hyperlink_protocol::metrics::AmbientBenchStats,
) -> BenchReport {
    BenchReport {
        timestamp: Utc::now().to_rfc3339(),
        protocol_version: PROTOCOL_VERSION,
        transport: transport_name.to_string(),
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
        ambient_stats: Some(ambient_stats),
        targets: TargetMetrics::default(),
    }
}

/// Print a human-readable summary to the terminal.
pub fn print_summary(report: &BenchReport) {
    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║           HyperLink Bench — Measurement Report          ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();
    println!("  Timestamp:  {}", report.timestamp);
    println!("  Protocol:   v{}", report.protocol_version);
    println!("  Transport:  {}", report.transport);
    println!();

    // Clock sync results.
    if let Some(ref cs) = report.clock_sync {
        println!("┌─── Clock Synchronization ───────────────────────────────┐");
        println!(
            "│  Rounds:          {:>10}                           │",
            cs.rounds
        );
        println!(
            "│  Best offset:     {:>10} µs                       │",
            cs.offset_us
        );
        println!(
            "│  Min RTT:         {:>10} µs                       │",
            cs.min_rtt_us
        );
        println!(
            "│  Mean RTT:        {:>10} µs                       │",
            cs.mean_rtt_us
        );
        println!(
            "│  Drift:           {:>10.2} µs/s                     │",
            cs.drift_us_per_sec
        );
        println!(
            "│  Drift (60s):     {:>10.2} µs                       │",
            cs.drift_us_per_sec * 60.0
        );
        let status = if cs.drift_within_spec {
            "✓ PASS (<1ms/60s)"
        } else {
            "✗ FAIL (≥1ms/60s)"
        };
        println!("│  DoD check:       {:<39} │", status);
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    // Echo results.
    if let Some(ref echo) = report.echo_stats {
        println!("┌─── Echo Latency ───────────────────────────────────────┐");
        println!(
            "│  Packets sent:    {:>10}                           │",
            echo.count + echo.lost
        );
        println!(
            "│  Packets recv:    {:>10}                           │",
            echo.count
        );
        println!(
            "│  Packets lost:    {:>10}                           │",
            echo.lost
        );
        println!("│                                                        │");
        println!(
            "│  Min RTT:         {:>10} µs                       │",
            echo.min_us
        );
        println!(
            "│  Max RTT:         {:>10} µs                       │",
            echo.max_us
        );
        println!(
            "│  Mean RTT:        {:>10.1} µs                       │",
            echo.mean_us
        );
        println!(
            "│  p50 RTT:         {:>10} µs                       │",
            echo.p50_us
        );
        println!(
            "│  p95 RTT:         {:>10} µs                       │",
            echo.p95_us
        );
        println!(
            "│  p99 RTT:         {:>10} µs                       │",
            echo.p99_us
        );
        println!(
            "│  Std dev:         {:>10.1} µs                       │",
            echo.stddev_us
        );
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    // Video streaming results.
    if let Some(ref video) = report.video_stats {
        println!("┌─── Video Stream Benchmark (Phase 2) ────────────────────┐");
        println!(
            "│  Frames sent:     {:>10}                           │",
            video.total_frames_sent
        );
        println!(
            "│  Frames received: {:>10}                           │",
            video.total_frames_received
        );
        println!(
            "│  Frames dropped:  {:>10} ({:>5.1}%)                 │",
            video.total_frames_dropped, video.frame_drop_rate_pct
        );
        println!(
            "│  Achieved FPS:    {:>10.1}                           │",
            video.achieved_fps
        );
        println!(
            "│  Avg Bitrate:     {:>10.1} kbps                      │",
            video.average_bitrate_kbps
        );
        if let Some(loss) = video.loss_simulated_pct {
            println!(
                "│  Injected loss:   {:>10.1} %                          │",
                loss
            );
        }
        println!("│  ── Latency Percentiles (One-Way) ──                    │");
        println!(
            "│  Min:             {:>10} µs ({:>6.2} ms)           │",
            video.latency_stats.min_us,
            video.latency_stats.min_us as f64 / 1000.0
        );
        println!(
            "│  Mean:            {:>10.1} µs ({:>6.2} ms)           │",
            video.latency_stats.mean_us,
            video.latency_stats.mean_us / 1000.0
        );
        println!(
            "│  p50 (Median):    {:>10} µs ({:>6.2} ms)           │",
            video.latency_stats.p50_us,
            video.latency_stats.p50_us as f64 / 1000.0
        );
        println!(
            "│  p95:             {:>10} µs ({:>6.2} ms)           │",
            video.latency_stats.p95_us,
            video.latency_stats.p95_us as f64 / 1000.0
        );
        println!(
            "│  p99:             {:>10} µs ({:>6.2} ms)           │",
            video.latency_stats.p99_us,
            video.latency_stats.p99_us as f64 / 1000.0
        );
        println!(
            "│  Phase 2 DoD Met: {:>10}                           │",
            if video.target_met {
                "YES (PASS)"
            } else {
                "NO (FAIL)"
            }
        );
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    if let Some(ref input) = report.input_stats {
        println!("┌─── Input Latency Benchmark (Phase 3) ───────────────────┐");
        println!(
            "│  Events dispatched:    {:>10}                           │",
            input.total_events
        );
        println!(
            "│  Acks received:        {:>10}                           │",
            input.total_acks
        );
        println!(
            "│  Events lost:          {:>10} ({:>5.1}%)                 │",
            input.lost_events, input.loss_rate_pct
        );
        println!("│  ── Latency Percentiles (Round-Trip) ──                 │");
        println!(
            "│  Min:                  {:>10} µs ({:>6.2} ms)           │",
            input.rtt_stats.min_us,
            input.rtt_stats.min_us as f64 / 1000.0
        );
        println!(
            "│  Mean:                 {:>10.1} µs ({:>6.2} ms)           │",
            input.rtt_stats.mean_us,
            input.rtt_stats.mean_us / 1000.0
        );
        println!(
            "│  p50 (Median):         {:>10} µs ({:>6.2} ms)           │",
            input.rtt_stats.p50_us,
            input.rtt_stats.p50_us as f64 / 1000.0
        );
        println!(
            "│  p95:                  {:>10} µs ({:>6.2} ms)           │",
            input.rtt_stats.p95_us,
            input.rtt_stats.p95_us as f64 / 1000.0
        );
        println!(
            "│  p99:                  {:>10} µs ({:>6.2} ms)           │",
            input.rtt_stats.p99_us,
            input.rtt_stats.p99_us as f64 / 1000.0
        );
        println!(
            "│  Phase 3 DoD Met: {:>10}                           │",
            if input.target_met {
                "YES (PASS)"
            } else {
                "NO (FAIL)"
            }
        );
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    if let Some(ref notif) = report.notification_stats {
        println!("┌─── Notification Sync Benchmark (Phase 4) ───────────────┐");
        println!(
            "│  Notifications sent:   {:>10}                           │",
            notif.total_posted
        );
        println!(
            "│  Acks received:        {:>10}                           │",
            notif.total_acks
        );
        println!(
            "│  Lost / Unacked:       {:>10} ({:>5.1}%)                 │",
            notif.lost_count, notif.loss_rate_pct
        );
        println!("│  ── Propagation Latency Percentiles ──                  │");
        println!(
            "│  Min:                  {:>10} µs ({:>6.2} ms)           │",
            notif.propagation_latency_stats.min_us,
            notif.propagation_latency_stats.min_us as f64 / 1000.0
        );
        println!(
            "│  Mean:                 {:>10.1} µs ({:>6.2} ms)           │",
            notif.propagation_latency_stats.mean_us,
            notif.propagation_latency_stats.mean_us / 1000.0
        );
        println!(
            "│  p50 (Median):         {:>10} µs ({:>6.2} ms)           │",
            notif.propagation_latency_stats.p50_us,
            notif.propagation_latency_stats.p50_us as f64 / 1000.0
        );
        println!(
            "│  p95:                  {:>10} µs ({:>6.2} ms)           │",
            notif.propagation_latency_stats.p95_us,
            notif.propagation_latency_stats.p95_us as f64 / 1000.0
        );
        println!(
            "│  p99:                  {:>10} µs ({:>6.2} ms)           │",
            notif.propagation_latency_stats.p99_us,
            notif.propagation_latency_stats.p99_us as f64 / 1000.0
        );
        if let Some(ref action_stats) = notif.action_rtt_stats {
            println!("│  ── Action Invocation Round-Trip ──                     │");
            println!(
                "│  Actions invoked:      {:>10}                           │",
                notif.total_actions_invoked
            );
            println!(
                "│  Action p50 RTT:       {:>10} µs ({:>6.2} ms)           │",
                action_stats.p50_us,
                action_stats.p50_us as f64 / 1000.0
            );
            println!(
                "│  Action p95 RTT:       {:>10} µs ({:>6.2} ms)           │",
                action_stats.p95_us,
                action_stats.p95_us as f64 / 1000.0
            );
        }
        println!(
            "│  Phase 4 DoD Met:      {:>10}                           │",
            if notif.target_met {
                "YES (PASS)"
            } else {
                "NO (FAIL)"
            }
        );
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    if let Some(ref clip) = report.clipboard_stats {
        println!("┌─── Clipboard Sync Benchmark (Phase 5) ─────────────────┐");
        println!(
            "│  Total syncs:          {:>10}                           │",
            clip.total_syncs
        );
        println!(
            "│  Acks received:        {:>10}                           │",
            clip.total_acks
        );
        println!(
            "│  Text syncs:           {:>10}                           │",
            clip.text_syncs
        );
        println!(
            "│  Image syncs:          {:>10}                           │",
            clip.image_syncs
        );
        println!(
            "│  Total data:           {:>10.2} MB                        │",
            clip.total_bytes as f64 / (1024.0 * 1024.0)
        );
        println!("│  ── Propagation Latency Percentiles ──                  │");
        println!(
            "│  Min:                  {:>10} µs ({:>6.2} ms)           │",
            clip.sync_latency_stats.min_us,
            clip.sync_latency_stats.min_us as f64 / 1000.0
        );
        println!(
            "│  Mean:                 {:>10.1} µs ({:>6.2} ms)           │",
            clip.sync_latency_stats.mean_us,
            clip.sync_latency_stats.mean_us / 1000.0
        );
        println!(
            "│  p50 (Median):         {:>10} µs ({:>6.2} ms)           │",
            clip.sync_latency_stats.p50_us,
            clip.sync_latency_stats.p50_us as f64 / 1000.0
        );
        println!(
            "│  p95:                  {:>10} µs ({:>6.2} ms)           │",
            clip.sync_latency_stats.p95_us,
            clip.sync_latency_stats.p95_us as f64 / 1000.0
        );
        println!(
            "│  p99:                  {:>10} µs ({:>6.2} ms)           │",
            clip.sync_latency_stats.p99_us,
            clip.sync_latency_stats.p99_us as f64 / 1000.0
        );
        println!("│  ── Loop Prevention & Multiplexing ──                   │");
        println!(
            "│  Echo loops detected:  {:>10}                           │",
            clip.loop_echoes_detected
        );
        println!(
            "│  Loop prevention:      {:>10}                           │",
            if clip.loop_prevention_passed {
                "PASS"
            } else {
                "FAIL"
            }
        );
        if let Some(p95_input) = clip.concurrent_input_p95_us {
            println!(
                "│  Concurrent input p95: {:>10} µs ({:>6.2} ms)           │",
                p95_input,
                p95_input as f64 / 1000.0
            );
        }
        println!(
            "│  Phase 5 DoD Met:      {:>10}                           │",
            if clip.target_met {
                "YES (PASS)"
            } else {
                "NO (FAIL)"
            }
        );
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    if let Some(ref file) = report.file_stats {
        println!("┌─── File Access & Virtual Mount Benchmark (Phase 6) ─────┐");
        println!(
            "│  Total reads:          {:>10}                           │",
            file.total_reads
        );
        println!(
            "│  Total writes:         {:>10}                           │",
            file.total_writes
        );
        println!(
            "│  Bytes read:           {:>10.2} MB                        │",
            file.total_bytes_read as f64 / (1024.0 * 1024.0)
        );
        println!(
            "│  Bytes written:        {:>10.2} KB                        │",
            file.total_bytes_written as f64 / 1024.0
        );
        println!(
            "│  Throughput:           {:>10.2} MB/s                      │",
            file.throughput_mbps
        );
        println!("│  ── Time-To-First-Byte (TTFB) Percentiles ──            │");
        println!(
            "│  Min:                  {:>10} µs ({:>6.2} ms)           │",
            file.ttfb_stats.min_us,
            file.ttfb_stats.min_us as f64 / 1000.0
        );
        println!(
            "│  Mean:                 {:>10.1} µs ({:>6.2} ms)           │",
            file.ttfb_stats.mean_us,
            file.ttfb_stats.mean_us / 1000.0
        );
        println!(
            "│  p50 (Median):         {:>10} µs ({:>6.2} ms)           │",
            file.ttfb_stats.p50_us,
            file.ttfb_stats.p50_us as f64 / 1000.0
        );
        println!(
            "│  p95:                  {:>10} µs ({:>6.2} ms)           │",
            file.ttfb_stats.p95_us,
            file.ttfb_stats.p95_us as f64 / 1000.0
        );
        println!(
            "│  p99:                  {:>10} µs ({:>6.2} ms)           │",
            file.ttfb_stats.p99_us,
            file.ttfb_stats.p99_us as f64 / 1000.0
        );
        println!("│  ── Verification Checks ──                              │");
        println!(
            "│  Lazy seek verified:   {:>10}                           │",
            if file.lazy_seek_verified {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Write-back verified:  {:>10}                           │",
            if file.writeback_verified {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Checksum byte match:  {:>10}                           │",
            if file.checksum_verified {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Phase 6 DoD Met:      {:>10}                           │",
            if file.target_met {
                "YES (PASS)"
            } else {
                "NO (FAIL)"
            }
        );
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    // Phase 7: Network Resilience & Multipath summary table
    if let Some(ref res) = report.resilience_stats {
        println!("┌─── Network Resilience & Multipath Benchmark (Phase 7) ──┐");
        println!(
            "│  Failovers tested:     {:>10}                           │",
            res.total_failovers_tested
        );
        println!(
            "│  Successful failovers: {:>10}                           │",
            res.successful_failovers
        );
        println!(
            "│  Detection latency:    {:>10.2} ms                       │",
            res.failover_detection_time_ms
        );
        println!(
            "│  Switch latency:       {:>10.2} ms                       │",
            res.failover_switch_time_ms
        );
        println!(
            "│  Total failover time:  {:>10.2} ms                       │",
            res.total_failover_latency_ms
        );
        println!("│  ── Stream Survival Checks ──                           │");
        println!(
            "│  Video stream survived:{:>10}                           │",
            if res.video_stream_survived {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Input stream survived:{:>10} (0 lost)                  │",
            if res.input_stream_survived {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Zero re-pairing:      {:>10}                           │",
            if res.zero_repairing_verified {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Phase 7 DoD Met:      {:>10}                           │",
            if res.target_met {
                "YES (PASS)"
            } else {
                "NO (FAIL)"
            }
        );
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    // Phase 8: Proximity & Pre-Warmed Connect summary table
    if let Some(ref prox) = report.proximity_stats {
        println!("┌─── Proximity & Pre-Warmed Connect (Phase 8) ────────────┐");
        println!(
            "│  Cold setup latency:   {:>10.2} ms                       │",
            prox.cold_setup_latency_ms
        );
        println!(
            "│  Pre-warmed setup:     {:>10.2} ms                       │",
            prox.prewarmed_setup_latency_ms
        );
        println!(
            "│  Latency reduction:    {:>10.1} %                        │",
            prox.latency_reduction_pct
        );
        println!(
            "│  In-range to mirror:   {:>10.2} ms                       │",
            prox.time_from_in_range_to_mirror_ms
        );
        println!("│  ── Verification Checks ──                              │");
        println!(
            "│  Workflow restored:    {:>10}                           │",
            if prox.workflow_state_restored {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Auth bypass prevented:{:>10}                           │",
            if prox.security_auth_bypass_prevented {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Phase 8 DoD Met:      {:>10}                           │",
            if prox.target_met {
                "YES (PASS)"
            } else {
                "NO (FAIL)"
            }
        );
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    // Phase 9: Scoped App-State Handoff summary table
    if let Some(ref handoff) = report.handoff_stats {
        println!("┌─── Scoped App-State Handoff (Phase 9) ──────────────────┐");
        println!(
            "│  Handoffs tested:      {:>10}                           │",
            handoff.total_handoffs_tested
        );
        println!(
            "│  Handoffs successful:  {:>10}                           │",
            handoff.successful_handoffs
        );
        println!(
            "│  Mean latency:         {:>10.2} ms                       │",
            handoff.avg_handoff_latency_ms
        );
        println!(
            "│  p50 latency:          {:>10.2} ms                       │",
            handoff.p50_latency_us as f64 / 1000.0
        );
        println!(
            "│  p95 latency:          {:>10.2} ms                       │",
            handoff.p95_latency_us as f64 / 1000.0
        );
        println!(
            "│  p99 latency:          {:>10.2} ms                       │",
            handoff.p99_latency_us as f64 / 1000.0
        );
        println!("│  ── Verification Checks ──                              │");
        println!(
            "│  State fidelity match: {:>10}                           │",
            if handoff.state_restoration_fidelity {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Schema verified:      {:>10}                           │",
            if handoff.contract_schema_verified {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Phase 9 DoD Met:      {:>10}                           │",
            if handoff.target_met {
                "YES (PASS)"
            } else {
                "NO (FAIL)"
            }
        );
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    // Phase 10: Ambient Context Agent summary table
    if let Some(ref amb) = report.ambient_stats {
        println!("┌─── Ambient Context Agent (Phase 10) ────────────────────┐");
        println!(
            "│  Events published:     {:>10}                           │",
            amb.total_events_published
        );
        println!(
            "│  Events delivered:     {:>10}                           │",
            amb.total_events_delivered
        );
        println!(
            "│  Mean publish latency: {:>10.2} µs                       │",
            amb.avg_publish_latency_us
        );
        println!(
            "│  p50 publish latency:  {:>10.2} µs                       │",
            amb.p50_latency_us as f64
        );
        println!(
            "│  p95 publish latency:  {:>10.2} µs                       │",
            amb.p95_latency_us as f64
        );
        println!("│  ── Security & Invariant Checks ──                      │");
        println!(
            "│  Consent gating 100%:  {:>10}                           │",
            if amb.consent_gating_verified {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Raw video sandboxed:  {:>10}                           │",
            if amb.raw_video_sandboxed {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Query timeline match: {:>10}                           │",
            if amb.agent_query_fidelity {
                "PASS"
            } else {
                "FAIL"
            }
        );
        println!(
            "│  Phase 10 DoD Met:     {:>10}                           │",
            if amb.target_met {
                "YES (PASS)"
            } else {
                "NO (FAIL)"
            }
        );
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }

    // Target metrics table.
    println!("┌─── Target Metrics (from SYSTEM_DESIGN.md) ──────────────┐");
    println!("│  Metric                    USB      5GHz     6GHz       │");
    println!("│  ─────────────────────     ─────    ─────    ─────      │");
    print_target_row(
        "Video glass-to-glass",
        &report.targets.video_latency.usb,
        &report.targets.video_latency.wifi_5ghz,
        &report.targets.video_latency.wifi_6ghz,
        "µs",
    );
    print_target_row(
        "Input RTT",
        &report.targets.input_rtt.usb,
        &report.targets.input_rtt.wifi_5ghz,
        &report.targets.input_rtt.wifi_6ghz,
        "µs",
    );
    print_target_row(
        "Notification",
        &report.targets.notification_latency.usb,
        &report.targets.notification_latency.wifi_5ghz,
        &report.targets.notification_latency.wifi_6ghz,
        "µs",
    );
    print_target_row(
        "Clipboard",
        &report.targets.clipboard_latency.usb,
        &report.targets.clipboard_latency.wifi_5ghz,
        &report.targets.clipboard_latency.wifi_6ghz,
        "µs",
    );
    print_target_row(
        "File throughput",
        &report.targets.file_throughput_mbps.usb,
        &report.targets.file_throughput_mbps.wifi_5ghz,
        &report.targets.file_throughput_mbps.wifi_6ghz,
        "MB/s",
    );
    print_target_row(
        "File TTFB",
        &report.targets.file_ttfb.usb,
        &report.targets.file_ttfb.wifi_5ghz,
        &report.targets.file_ttfb.wifi_6ghz,
        "µs",
    );
    println!("└─────────────────────────────────────────────────────────┘");
    println!();
}

fn print_target_row(
    name: &str,
    usb: &Option<f64>,
    wifi5: &Option<f64>,
    wifi6: &Option<f64>,
    unit: &str,
) {
    let fmt = |v: &Option<f64>| match v {
        Some(val) => format!("{:.0}{}", val, unit),
        None => "TBD".to_string(),
    };
    println!(
        "│  {:<24} {:<8} {:<8} {:<10} │",
        name,
        fmt(usb),
        fmt(wifi5),
        fmt(wifi6),
    );
}

/// Write the report as a JSON line to the results file.
///
/// Creates the results directory if it doesn't exist.
pub fn write_to_file(report: &BenchReport, results_dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(results_dir)?;

    let filename = format!("bench_{}.jsonl", Utc::now().format("%Y%m%d_%H%M%S"));
    let path = results_dir.join(&filename);

    let json = serde_json::to_string(report)?;
    std::fs::write(&path, format!("{}\n", json))?;

    info!(path = %path.display(), "report written to file");
    println!("  Report saved to: {}", path.display());

    Ok(path)
}

/// Write the full report as pretty-printed JSON to stdout.
pub fn print_json(report: &BenchReport) {
    match serde_json::to_string_pretty(report) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("Failed to serialize report: {e}"),
    }
}
