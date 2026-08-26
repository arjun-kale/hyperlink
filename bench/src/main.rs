//! HyperLink bench — measurement harness entry point.
//!
//! Phase 0: clock-sync handshake + round-trip echo.
//! See `docs/SYSTEM_DESIGN.md` for the phase-by-phase build order.
//!
//! # Usage
//!
//! ```sh
//! # Terminal 1: Start the bench server
//! hyperlink-bench server --bind 127.0.0.1:9900
//!
//! # Terminal 2: Run the full benchmark suite
//! hyperlink-bench client --target 127.0.0.1:9900
//!
//! # Client with custom parameters
//! hyperlink-bench client --target 127.0.0.1:9900 \
//!     --clock-sync-rounds 16 \
//!     --echo-count 200 \
//!     --drift-window 60 \
//!     --json
//! ```

mod ambient_bench;
mod clipboard_bench;
mod clock_sync;
mod echo;
mod file_bench;
mod handoff_bench;
mod input_bench;
mod notification_bench;
mod proximity_bench;
mod report;
mod resilience_bench;
mod server;
mod transport;
mod video_bench;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use transport::UdpTransport;

/// HyperLink measurement harness — clock sync, echo latency, video, input, notifications, clipboard.
#[derive(Parser)]
#[command(name = "hyperlink-bench")]
#[command(version)]
#[command(about = "HyperLink measurement harness — Phases 0, 2-10")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the bench server (responds to clock sync and echo requests).
    Server {
        /// Address to bind the server to.
        #[arg(short, long, default_value = "0.0.0.0:9900")]
        bind: SocketAddr,
    },

    /// Run the bench client (initiates clock sync and echo tests).
    Client {
        /// Server address to connect to.
        #[arg(short, long)]
        target: SocketAddr,

        /// Number of clock sync rounds.
        #[arg(long, default_value_t = 8)]
        clock_sync_rounds: u32,

        /// Duration in seconds for drift measurement window.
        #[arg(long, default_value_t = 60.0)]
        drift_window: f64,

        /// Number of echo packets to send.
        #[arg(long, default_value_t = 100)]
        echo_count: u32,

        /// Interval between echo packets in milliseconds.
        #[arg(long, default_value_t = 50)]
        echo_interval: u64,

        /// Payload size in bytes for echo packets (tests throughput under load).
        #[arg(long, default_value_t = 0)]
        payload_size: u32,

        /// Timeout per echo packet in milliseconds.
        #[arg(long, default_value_t = 2000)]
        timeout: u64,

        /// Output full report as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,

        /// Skip the clock sync phase.
        #[arg(long)]
        skip_clock_sync: bool,

        /// Skip the echo phase.
        #[arg(long)]
        skip_echo: bool,

        /// Directory to write results files.
        #[arg(long, default_value = "bench/results")]
        results_dir: PathBuf,
    },

    /// Run the Phase 2 video streaming benchmark.
    Video {
        /// Number of video frames to simulate.
        #[arg(long, default_value_t = 300)]
        frames: u32,

        /// Target frame rate in FPS.
        #[arg(long, default_value_t = 30)]
        fps: u32,

        /// Target bitrate in kbps.
        #[arg(long, default_value_t = 4000)]
        bitrate_kbps: u32,

        /// Simulated packet loss rate (0.0 to 1.0, e.g. 0.05 for 5% loss).
        #[arg(long, default_value_t = 0.0)]
        loss_rate: f64,

        /// Output full report as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,

        /// Directory to write results files.
        #[arg(long, default_value = "bench/results")]
        results_dir: PathBuf,
    },

    /// Run the Phase 3 input latency benchmark.
    Input {
        /// Number of input events to simulate.
        #[arg(long, default_value_t = 200)]
        count: u64,

        /// Interval between input events in milliseconds (default: 8ms = ~125Hz).
        #[arg(long, default_value_t = 8)]
        interval_ms: u64,

        /// Simulated packet loss rate (0.0 to 1.0).
        #[arg(long, default_value_t = 0.0)]
        loss_rate: f64,

        /// Output full report as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,

        /// Directory to write results files.
        #[arg(long, default_value = "bench/results")]
        results_dir: PathBuf,
    },

    /// Run the Phase 4 notification latency benchmark.
    Notification {
        /// Number of notifications to dispatch.
        #[arg(long, default_value_t = 50)]
        count: u64,

        /// Interval between notifications in milliseconds.
        #[arg(long, default_value_t = 20)]
        interval_ms: u64,

        /// Number of interactive action button clicks to simulate.
        #[arg(long, default_value_t = 10)]
        actions: u64,

        /// Whether to include 48x48 icon byte payloads.
        #[arg(long, default_value_t = true)]
        icons: bool,

        /// Simulated packet loss rate (0.0 to 1.0).
        #[arg(long, default_value_t = 0.0)]
        loss_rate: f64,

        /// Output full report as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,

        /// Directory to write results files.
        #[arg(long, default_value = "bench/results")]
        results_dir: PathBuf,
    },

    /// Run the Phase 5 clipboard sync benchmark.
    Clipboard {
        /// Number of clipboard syncs to simulate.
        #[arg(long, default_value_t = 30)]
        count: u64,

        /// Interval between clipboard syncs in milliseconds.
        #[arg(long, default_value_t = 20)]
        interval_ms: u64,

        /// Whether to include image payloads (e.g. 2 MB - 5 MB image payloads).
        #[arg(long, default_value_t = true)]
        images: bool,

        /// Custom image size in KB.
        #[arg(long, default_value_t = 2048)]
        image_size_kb: u64,

        /// Test loop prevention and echo rejection.
        #[arg(long, default_value_t = true)]
        test_loops: bool,

        /// Test concurrent input stream multiplexing under large clipboard payload.
        #[arg(long, default_value_t = true)]
        concurrent_input: bool,

        /// Simulated packet loss rate (0.0 to 1.0).
        #[arg(long, default_value_t = 0.0)]
        loss_rate: f64,

        /// Output full report as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,

        /// Directory to write results files.
        #[arg(long, default_value = "bench/results")]
        results_dir: PathBuf,
    },

    /// Run the Phase 6 file access & virtual mount benchmark.
    File {
        /// Simulated file size in MB (e.g. 500 MB video).
        #[arg(long, default_value_t = 500)]
        file_size_mb: u64,

        /// Chunk size in KB.
        #[arg(long, default_value_t = 64)]
        chunk_size_kb: u32,

        /// Number of random seek read operations to simulate (scrubbing).
        #[arg(long, default_value_t = 20)]
        seeks: u64,

        /// Whether to verify lazy seeking (asserting no upfront download).
        #[arg(long, default_value_t = true)]
        lazy_seek: bool,

        /// Whether to verify write-back of modified chunks to storage.
        #[arg(long, default_value_t = true)]
        writeback: bool,

        /// Whether to verify byte-for-byte SHA-256 checksum integrity of full chunked read.
        #[arg(long, default_value_t = true)]
        checksum: bool,

        /// Simulated packet loss rate (0.0 to 1.0).
        #[arg(long, default_value_t = 0.0)]
        loss_rate: f64,

        /// Output full report as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,

        /// Directory to write results files.
        #[arg(long, default_value = "bench/results")]
        results_dir: PathBuf,
    },

    /// Run the Phase 7 network resilience & multipath benchmark.
    Resilience {
        /// Number of failover scenarios to execute.
        #[arg(long, default_value_t = 3)]
        scenarios: u64,

        /// Number of video frames sent per scenario.
        #[arg(long, default_value_t = 100)]
        frames: u64,

        /// Number of input events dispatched per scenario.
        #[arg(long, default_value_t = 50)]
        input_events: u64,

        /// Simulated packet loss rate on standby path (0.0 to 1.0).
        #[arg(long, default_value_t = 0.0)]
        loss_rate: f64,

        /// Whether to buffer input events during route failover.
        #[arg(long, default_value_t = true)]
        buffer_input: bool,

        /// Target maximum allowable failover latency in milliseconds.
        #[arg(long, default_value_t = 1000.0)]
        max_failover_ms: f64,

        /// Output full report as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,

        /// Directory to write results files.
        #[arg(long, default_value = "bench/results")]
        results_dir: PathBuf,
    },

    /// Run the Phase 8 proximity ranging, pre-warmed connect, and workflow restore benchmark.
    Proximity {
        /// Number of proximity cycles to execute.
        #[arg(long, default_value_t = 5)]
        iterations: u64,

        /// Distance in centimeters.
        #[arg(long, default_value_t = 45)]
        distance_cm: u16,

        /// Target maximum allowable pre-warmed setup latency in milliseconds.
        #[arg(long, default_value_t = 50.0)]
        max_latency_ms: f64,

        /// Output full report as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,

        /// Directory to write results files.
        #[arg(long, default_value = "bench/results")]
        results_dir: PathBuf,
    },

    /// Run the Phase 9 scoped app-state handoff benchmark suite.
    Handoff {
        /// Number of handoff cycles to test.
        #[arg(long, default_value_t = 10)]
        iterations: u64,

        /// Target maximum allowable 95th percentile latency in milliseconds.
        #[arg(long, default_value_t = 100.0)]
        max_p95_ms: f64,

        /// Output full report as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,

        /// Directory to write results files.
        #[arg(long, default_value = "bench/results")]
        results_dir: PathBuf,
    },

    /// Run the Phase 10 ambient context agent benchmark suite.
    Ambient {
        /// Number of telemetry event cycles to test.
        #[arg(long, default_value_t = 20)]
        iterations: u64,

        /// Target maximum allowable 95th percentile publish latency in milliseconds.
        #[arg(long, default_value_t = 10.0)]
        max_p95_ms: f64,

        /// Output full report as JSON instead of human-readable format.
        #[arg(long)]
        json: bool,

        /// Directory to write results files.
        #[arg(long, default_value = "bench/results")]
        results_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() {
    // Initialize structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Server { bind } => {
            if let Err(e) = run_server(bind).await {
                error!(error = %e, "server failed");
                std::process::exit(1);
            }
        }
        Commands::Client {
            target,
            clock_sync_rounds,
            drift_window,
            echo_count,
            echo_interval,
            payload_size,
            timeout,
            json,
            skip_clock_sync,
            skip_echo,
            results_dir,
        } => {
            if let Err(e) = run_client(
                target,
                clock_sync_rounds,
                drift_window,
                echo_count,
                echo_interval,
                payload_size,
                timeout,
                json,
                skip_clock_sync,
                skip_echo,
                results_dir,
            )
            .await
            {
                error!(error = %e, "client failed");
                std::process::exit(1);
            }
        }
        Commands::Video {
            frames,
            fps,
            bitrate_kbps,
            loss_rate,
            json,
            results_dir,
        } => {
            let options = video_bench::VideoBenchOptions {
                total_frames: frames,
                target_fps: fps,
                target_bitrate_kbps: bitrate_kbps,
                simulated_loss_rate: loss_rate,
                ..Default::default()
            };
            let video_stats = video_bench::run_video_benchmark(&options);
            let bench_report = report::build_video_report("quic-datagrams", video_stats);
            if json {
                report::print_json(&bench_report);
            } else {
                report::print_summary(&bench_report);
            }
            if let Err(e) = report::write_to_file(&bench_report, &results_dir) {
                error!("failed to write report: {}", e);
            }
        }
        Commands::Input {
            count,
            interval_ms,
            loss_rate,
            json,
            results_dir,
        } => {
            let options = input_bench::InputBenchOptions {
                total_events: count,
                interval_ms,
                simulated_loss_rate: loss_rate,
            };
            let input_stats = input_bench::run_input_benchmark(&options);
            let bench_report = report::build_input_report("quic-input-stream", input_stats);
            if json {
                report::print_json(&bench_report);
            } else {
                report::print_summary(&bench_report);
            }
            if let Err(e) = report::write_to_file(&bench_report, &results_dir) {
                error!("failed to write report: {}", e);
            }
        }
        Commands::Notification {
            count,
            interval_ms,
            actions,
            icons,
            loss_rate,
            json,
            results_dir,
        } => {
            let options = notification_bench::NotificationBenchOptions {
                total_notifications: count,
                interval_ms,
                action_invocations: actions,
                include_icons: icons,
                simulated_loss_rate: loss_rate,
            };
            let notif_stats = notification_bench::run_notification_benchmark(&options);
            let bench_report =
                report::build_notification_report("quic-control-stream", notif_stats);
            if json {
                report::print_json(&bench_report);
            } else {
                report::print_summary(&bench_report);
            }
            if let Err(e) = report::write_to_file(&bench_report, &results_dir) {
                error!("failed to write report: {}", e);
            }
        }
        Commands::Clipboard {
            count,
            interval_ms,
            images,
            image_size_kb,
            test_loops,
            concurrent_input,
            loss_rate,
            json,
            results_dir,
        } => {
            let options = clipboard_bench::ClipboardBenchOptions {
                total_syncs: count,
                interval_ms,
                include_images: images,
                image_size_kb,
                test_loop_prevention: test_loops,
                concurrent_input,
                simulated_loss_rate: loss_rate,
            };
            let clip_stats = clipboard_bench::run_clipboard_benchmark(&options);
            let bench_report = report::build_clipboard_report("quic-clipboard-stream", clip_stats);
            if json {
                report::print_json(&bench_report);
            } else {
                report::print_summary(&bench_report);
            }
            if let Err(e) = report::write_to_file(&bench_report, &results_dir) {
                error!("failed to write report: {}", e);
            }
        }
        Commands::File {
            file_size_mb,
            chunk_size_kb,
            seeks,
            lazy_seek,
            writeback,
            checksum,
            loss_rate,
            json,
            results_dir,
        } => {
            let options = file_bench::FileBenchOptions {
                file_size_mb,
                chunk_size_kb,
                random_seeks_count: seeks,
                test_lazy_seek: lazy_seek,
                test_writeback: writeback,
                test_checksum: checksum,
                simulated_loss_rate: loss_rate,
            };
            let file_stats = file_bench::run_file_benchmark(&options);
            let bench_report = report::build_file_report("quic-vfs-stream", file_stats);
            if json {
                report::print_json(&bench_report);
            } else {
                report::print_summary(&bench_report);
            }
            if let Err(e) = report::write_to_file(&bench_report, &results_dir) {
                error!("failed to write report: {}", e);
            }
        }
        Commands::Resilience {
            scenarios,
            frames,
            input_events,
            loss_rate,
            buffer_input,
            max_failover_ms,
            json,
            results_dir,
        } => {
            let options = resilience_bench::ResilienceBenchOptions {
                failover_scenarios: scenarios,
                frames_per_scenario: frames,
                input_events_per_scenario: input_events,
                simulated_loss_rate: loss_rate,
                buffer_input_during_transition: buffer_input,
                max_failover_latency_ms: max_failover_ms,
            };
            match resilience_bench::run_resilience_bench(options).await {
                Ok(resilience_stats) => {
                    let bench_report = report::build_resilience_report(
                        "quic-multipath-failover",
                        resilience_stats,
                    );
                    if json {
                        report::print_json(&bench_report);
                    } else {
                        report::print_summary(&bench_report);
                    }
                    if let Err(e) = report::write_to_file(&bench_report, &results_dir) {
                        error!("failed to write report: {}", e);
                    }
                }
                Err(e) => {
                    error!("resilience benchmark failed: {}", e);
                }
            }
        }
        Commands::Proximity {
            iterations,
            distance_cm,
            max_latency_ms,
            json,
            results_dir,
        } => {
            let options = proximity_bench::ProximityBenchOptions {
                iterations,
                technology: hyperlink_protocol::proximity::ProximityTechnology::UwbRanging,
                distance_cm,
                rssi_dbm: -42,
                confidence_pct: 98,
                simulate_untrusted_beacon: false,
                max_prewarmed_latency_ms: max_latency_ms,
            };
            match proximity_bench::run_proximity_bench(options).await {
                Ok(proximity_stats) => {
                    let bench_report =
                        report::build_proximity_report("uwb-proximity-prewarm", proximity_stats);
                    if json {
                        report::print_json(&bench_report);
                    } else {
                        report::print_summary(&bench_report);
                    }
                    if let Err(e) = report::write_to_file(&bench_report, &results_dir) {
                        error!("failed to write report: {}", e);
                    }
                }
                Err(e) => {
                    error!("proximity benchmark failed: {}", e);
                }
            }
        }
        Commands::Handoff {
            iterations,
            max_p95_ms,
            json,
            results_dir,
        } => {
            let options = handoff_bench::HandoffBenchOptions {
                iterations,
                max_p95_latency_ms: max_p95_ms,
                simulate_invalid_payload: false,
            };
            match handoff_bench::run_handoff_bench(options).await {
                Ok(handoff_stats) => {
                    let bench_report =
                        report::build_handoff_report("quic-state-handoff", handoff_stats);
                    if json {
                        report::print_json(&bench_report);
                    } else {
                        report::print_summary(&bench_report);
                    }
                    if let Err(e) = report::write_to_file(&bench_report, &results_dir) {
                        error!("failed to write report: {}", e);
                    }
                }
                Err(e) => {
                    error!("handoff benchmark failed: {}", e);
                }
            }
        }
        Commands::Ambient {
            iterations,
            max_p95_ms,
            json,
            results_dir,
        } => {
            let options = ambient_bench::AmbientBenchOptions {
                iterations,
                max_p95_latency_ms: max_p95_ms,
                simulate_consent_violation: false,
                simulate_video_leak: false,
            };
            match ambient_bench::run_ambient_bench(options).await {
                Ok(ambient_stats) => {
                    let bench_report =
                        report::build_ambient_report("ambient-context-bus", ambient_stats);
                    if json {
                        report::print_json(&bench_report);
                    } else {
                        report::print_summary(&bench_report);
                    }
                    if let Err(e) = report::write_to_file(&bench_report, &results_dir) {
                        error!("failed to write report: {}", e);
                    }
                }
                Err(e) => {
                    error!("ambient benchmark failed: {}", e);
                }
            }
        }
    }
}

async fn run_server(bind: SocketAddr) -> anyhow::Result<()> {
    info!(bind = %bind, "starting HyperLink bench server");

    let transport = UdpTransport::bind(bind).await?;
    let addr = transport.local_addr()?;
    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║           HyperLink Bench — Server Mode                 ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();
    println!("  Listening on: {addr}");
    println!("  Responding to: ClockSync, Echo");
    println!("  Press Ctrl+C to stop.");
    println!();

    // Unified server with a single recv loop dispatching by message type.
    tokio::select! {
        r = server::run(&transport) => {
            if let Err(e) = r {
                error!(error = %e, "server error");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("shutting down");
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_client(
    target: SocketAddr,
    clock_sync_rounds: u32,
    drift_window: f64,
    echo_count: u32,
    echo_interval: u64,
    payload_size: u32,
    timeout: u64,
    json_output: bool,
    skip_clock_sync: bool,
    skip_echo: bool,
    results_dir: PathBuf,
) -> anyhow::Result<()> {
    info!(target = %target, "starting HyperLink bench client");

    // Bind to any available port and connect to the server.
    let mut transport = UdpTransport::bind("0.0.0.0:0".parse().unwrap()).await?;
    transport.connect(target).await?;
    let local = transport.local_addr()?;
    info!(local = %local, target = %target, "connected");

    // Phase 1: Clock sync.
    let clock_sync_result = if !skip_clock_sync {
        info!("=== Clock Synchronization ===");
        Some(clock_sync::run_client(&transport, clock_sync_rounds, drift_window).await?)
    } else {
        info!("clock sync skipped");
        None
    };

    // Phase 2: Echo test.
    let (echo_stats, echo_measurements) = if !skip_echo {
        info!("=== Echo Latency Test ===");
        let (stats, measurements) =
            echo::run_client(&transport, echo_count, echo_interval, payload_size, timeout).await?;
        (Some(stats), measurements)
    } else {
        info!("echo test skipped");
        (None, vec![])
    };

    // Build and output report.
    let bench_report =
        report::build_report("udp", clock_sync_result, echo_stats, echo_measurements);

    if json_output {
        report::print_json(&bench_report);
    } else {
        report::print_summary(&bench_report);
    }

    // Always write to file.
    report::write_to_file(&bench_report, &results_dir)?;

    Ok(())
}
