//! NTP-style clock synchronization between bench client and server.
//!
//! Runs multiple rounds of timestamp exchange to estimate the clock offset,
//! then monitors drift over a configurable window (default: 60 seconds).

use std::time::Duration;

use chrono::Utc;
use tracing::{debug, info, warn};

use hyperlink_protocol::clock::{ClockSyncRequest, ClockSyncResponse, CLOCK_SYNC_REQUEST_SIZE};
use hyperlink_protocol::message::MessageType;
use hyperlink_protocol::metrics::ClockSyncResult;
use hyperlink_protocol::version::{Header, HEADER_SIZE};

use crate::transport::{Transport, UdpTransport};

/// Run clock sync as the client (initiator).
///
/// Performs `rounds` exchanges, then monitors drift over `drift_window` seconds.
/// Returns a `ClockSyncResult` with offset, RTT, and drift measurements.
pub async fn run_client(
    transport: &UdpTransport,
    rounds: u32,
    drift_window_secs: f64,
) -> std::io::Result<ClockSyncResult> {
    info!(rounds, drift_window_secs, "clock sync client: starting");

    // Phase 1: Initial offset estimation with multiple rounds.
    let mut offsets = Vec::with_capacity(rounds as usize);
    let mut rtts = Vec::with_capacity(rounds as usize);

    for seq in 0..rounds {
        let (offset, rtt) = single_round(transport, seq).await?;
        offsets.push(offset);
        rtts.push(rtt);
        debug!(seq, offset_us = offset, rtt_us = rtt, "clock sync round");

        // Small delay between rounds to avoid hammering.
        if seq < rounds - 1 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // Use the round with the minimum RTT for the best offset estimate
    // (less network jitter = more accurate offset).
    let best_idx = rtts
        .iter()
        .enumerate()
        .min_by_key(|(_, &rtt)| rtt)
        .map(|(i, _)| i)
        .unwrap();
    let best_offset = offsets[best_idx];
    let min_rtt = rtts[best_idx];
    let mean_rtt = rtts.iter().sum::<i64>() / rtts.len() as i64;

    info!(
        best_offset_us = best_offset,
        min_rtt_us = min_rtt,
        mean_rtt_us = mean_rtt,
        "clock sync: initial offset estimated"
    );

    // Phase 2: Drift measurement over the configured window.
    let drift_us_per_sec = measure_drift(transport, rounds, drift_window_secs).await?;
    let drift_over_60s = (drift_us_per_sec * 60.0).abs();
    let drift_within_spec = drift_over_60s < 1000.0; // 1ms = 1000µs

    if drift_within_spec {
        info!(
            drift_us_per_sec,
            drift_over_60s_us = drift_over_60s,
            "clock sync: drift WITHIN spec (<1ms/60s)"
        );
    } else {
        warn!(
            drift_us_per_sec,
            drift_over_60s_us = drift_over_60s,
            "clock sync: drift EXCEEDS spec (≥1ms/60s)"
        );
    }

    Ok(ClockSyncResult {
        rounds,
        offset_us: best_offset,
        min_rtt_us: min_rtt,
        mean_rtt_us: mean_rtt,
        drift_us_per_sec,
        window_secs: drift_window_secs,
        drift_within_spec,
    })
}

/// Perform a single clock sync round-trip. Returns (offset_us, rtt_us).
async fn single_round(transport: &UdpTransport, seq: u32) -> std::io::Result<(i64, i64)> {
    let t1_us = now_us();

    let request = ClockSyncRequest { seq, t1_us };
    let mut send_buf = Vec::with_capacity(HEADER_SIZE + CLOCK_SYNC_REQUEST_SIZE);
    let header = Header::new(
        MessageType::ClockSyncRequest,
        CLOCK_SYNC_REQUEST_SIZE as u32,
    );
    header.encode(&mut send_buf)?;
    request.encode(&mut send_buf)?;

    transport.send(&send_buf).await?;

    // Wait for response with timeout.
    let mut recv_buf = [0u8; 1500];
    let n = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (n, _) = transport.recv(&mut recv_buf).await?;
            if n < HEADER_SIZE {
                continue;
            }
            let h = Header::decode(&recv_buf[..n])?;
            if h.message_type == MessageType::ClockSyncResponse {
                return Ok::<usize, std::io::Error>(n);
            }
            // Not our message type, keep waiting.
        }
    })
    .await
    .map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::TimedOut, "clock sync response timeout")
    })??;

    let t4_us = now_us();

    let response = ClockSyncResponse::decode(&recv_buf[HEADER_SIZE..n])?;
    if response.seq != seq {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("sequence mismatch: expected {}, got {}", seq, response.seq),
        ));
    }

    let offset = response.compute_offset(t4_us);
    let delay = response.compute_delay(t4_us);

    Ok((offset, delay))
}

/// Measure clock drift by sampling offset at intervals over a window.
///
/// Returns drift in µs/second.
async fn measure_drift(
    transport: &UdpTransport,
    _initial_rounds: u32,
    window_secs: f64,
) -> std::io::Result<f64> {
    let num_samples = 10.max((window_secs * 2.0) as u32); // ~2 samples/sec
    let interval = Duration::from_secs_f64(window_secs / num_samples as f64);

    let mut samples: Vec<(f64, i64)> = Vec::with_capacity(num_samples as usize);
    let start = std::time::Instant::now();

    for i in 0..num_samples {
        let elapsed = start.elapsed().as_secs_f64();
        match single_round(transport, 1000 + i).await {
            Ok((offset, _rtt)) => {
                samples.push((elapsed, offset));
            }
            Err(e) => {
                warn!(sample = i, error = %e, "drift measurement: sample failed, skipping");
            }
        }

        if i < num_samples - 1 {
            tokio::time::sleep(interval).await;
        }
    }

    if samples.len() < 2 {
        warn!("drift measurement: insufficient samples, returning 0");
        return Ok(0.0);
    }

    // Simple linear regression: offset = drift * time + base_offset
    let n = samples.len() as f64;
    let sum_t: f64 = samples.iter().map(|(t, _)| t).sum();
    let sum_o: f64 = samples.iter().map(|(_, o)| *o as f64).sum();
    let sum_to: f64 = samples.iter().map(|(t, o)| t * (*o as f64)).sum();
    let sum_tt: f64 = samples.iter().map(|(t, _)| t * t).sum();

    let denominator = n * sum_tt - sum_t * sum_t;
    if denominator.abs() < f64::EPSILON {
        return Ok(0.0);
    }

    let drift = (n * sum_to - sum_t * sum_o) / denominator;

    info!(
        samples = samples.len(),
        window_secs,
        drift_us_per_sec = drift,
        "drift measurement complete"
    );

    Ok(drift)
}

/// Get the current time in microseconds since the Unix epoch.
fn now_us() -> i64 {
    Utc::now().timestamp_micros()
}
