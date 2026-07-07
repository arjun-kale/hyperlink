//! Report formatting and file output.
//!
//! Outputs bench results as structured JSON (for machine consumption) and
//! a human-readable table (for terminal display). Each run appends a
//! timestamped JSON line to a results file for historical tracking.

use std::path::{Path, PathBuf};

use chrono::Utc;
use tracing::info;

use hyperlink_protocol::metrics::{
    BenchReport, ClockSyncResult, LatencyMeasurement, LatencyStats, TargetMetrics,
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
