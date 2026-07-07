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

mod clock_sync;
mod echo;
mod report;
mod server;
mod transport;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use transport::UdpTransport;

/// HyperLink measurement harness — clock sync, echo latency, throughput.
#[derive(Parser)]
#[command(name = "hyperlink-bench")]
#[command(version)]
#[command(about = "HyperLink measurement harness — Phase 0")]
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
