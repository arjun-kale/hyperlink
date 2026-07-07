//! HyperLink Linux host daemon.
//!
//! Exposes a QUIC server, advertises service via mDNS, handles TOFU pairing
//! PIN confirmations, and manages paired connection sockets.

mod connection;
mod discovery;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use hyperlink_protocol::config::DeviceConfig;

#[derive(Parser)]
#[command(name = "hyperlink-linux")]
#[command(version)]
#[command(about = "HyperLink Linux Host Daemon")]
struct Cli {
    /// Address to bind the QUIC server to.
    #[arg(short, long, default_value = "0.0.0.0:9900")]
    bind: SocketAddr,

    /// Device display name advertised over mDNS.
    #[arg(short, long, default_value = "Linux-Host")]
    name: String,

    /// Start in pairing mode to pair a new client companion.
    #[arg(short, long)]
    pair: bool,

    /// Custom path to load/save host credentials and paired devices.
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    // Determine config path.
    let config_path = match cli.config {
        Some(p) => p,
        None => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config/hyperlink/host_config.json")
        }
    };

    info!("loading host configuration from: {:?}", config_path);
    let host_config = DeviceConfig::load_or_create(&config_path, &cli.name)?;

    // Start mDNS advertisement.
    let _discovery = match discovery::start_advertisement(&host_config.device_name, cli.bind.port()) {
        Ok(handle) => {
            info!("mDNS advertisement started successfully");
            Some(handle)
        }
        Err(e) => {
            error!("failed to start mDNS advertisement: {}", e);
            None
        }
    };

    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║           HyperLink Host Daemon — Phase 1               ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();
    println!("  Device Name:   {}", host_config.device_name);
    println!("  Listening on:  {}", cli.bind);
    println!("  Mode:          {}", if cli.pair { "Pairing Mode" } else { "Normal Mode" });
    println!("  Press Ctrl+C to stop.");
    println!();

    // Start the server and wait for connections.
    tokio::select! {
        res = connection::start_server(cli.bind, host_config, config_path, cli.pair) => {
            if let Err(e) = res {
                error!("server failed: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("shutting down server daemon");
        }
    }

    Ok(())
}
