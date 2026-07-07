//! Echo test for round-trip latency measurement.
//!
//! Sends timestamped packets to the server and measures RTT for each.
//! Produces aggregate statistics (min, max, mean, percentiles).

use std::time::Duration;

use chrono::Utc;
use tracing::{debug, info, warn};

use hyperlink_protocol::echo::{EchoRequest, EchoResponse, ECHO_REQUEST_FIXED_SIZE};
use hyperlink_protocol::message::MessageType;
use hyperlink_protocol::metrics::{LatencyMeasurement, LatencyStats};
use hyperlink_protocol::version::{Header, HEADER_SIZE};

use crate::transport::{Transport, UdpTransport};

/// Run echo test as the client (initiator).
///
/// Sends `count` echo requests at the given interval, measures RTT for each,
/// and returns aggregate statistics plus individual measurements.
pub async fn run_client(
    transport: &UdpTransport,
    count: u32,
    interval_ms: u64,
    payload_size: u32,
    timeout_ms: u64,
) -> std::io::Result<(LatencyStats, Vec<LatencyMeasurement>)> {
    info!(
        count,
        interval_ms, payload_size, timeout_ms, "echo client: starting"
    );

    let mut rtts: Vec<i64> = Vec::with_capacity(count as usize);
    let mut measurements: Vec<LatencyMeasurement> = Vec::with_capacity(count as usize);
    let mut lost: u64 = 0;

    for seq in 0..count {
        let send_time_us = now_us();

        let request = EchoRequest {
            seq,
            send_time_us,
            payload_size,
        };

        let mut send_buf = Vec::with_capacity(HEADER_SIZE + ECHO_REQUEST_FIXED_SIZE);
        let header = Header::new(MessageType::EchoRequest, ECHO_REQUEST_FIXED_SIZE as u32);
        header.encode(&mut send_buf)?;
        request.encode(&mut send_buf)?;

        // Append padding payload if requested.
        if payload_size > 0 {
            send_buf.extend(std::iter::repeat_n(0xAA, payload_size as usize));
        }

        transport.send(&send_buf).await?;

        // Wait for response with timeout.
        let result = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            wait_for_echo_response(transport, seq),
        )
        .await;

        match result {
            Ok(Ok(response)) => {
                let recv_time_us = now_us();
                let rtt_us = recv_time_us - response.send_time_us;
                let estimated_one_way_us = rtt_us / 2;

                rtts.push(rtt_us);
                measurements.push(LatencyMeasurement {
                    seq,
                    rtt_us,
                    estimated_one_way_us,
                    timestamp: Utc::now().to_rfc3339(),
                });

                debug!(
                    seq,
                    rtt_us, estimated_one_way_us, "echo client: received response"
                );
            }
            Ok(Err(e)) => {
                warn!(seq, error = %e, "echo client: error receiving response");
                lost += 1;
            }
            Err(_) => {
                warn!(seq, timeout_ms, "echo client: response timed out");
                lost += 1;
            }
        }

        // Wait before next packet (skip on last iteration).
        if seq < count - 1 {
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
    }

    if rtts.is_empty() {
        return Err(std::io::Error::other("all echo requests were lost"));
    }

    let stats = LatencyStats::from_rtts(&rtts, lost);

    info!(
        count = stats.count,
        lost = stats.lost,
        min_us = stats.min_us,
        max_us = stats.max_us,
        mean_us = format!("{:.1}", stats.mean_us),
        p50_us = stats.p50_us,
        p95_us = stats.p95_us,
        p99_us = stats.p99_us,
        "echo client: test complete"
    );

    Ok((stats, measurements))
}

/// Wait for an `EchoResponse` with a specific sequence number.
async fn wait_for_echo_response(
    transport: &UdpTransport,
    expected_seq: u32,
) -> std::io::Result<EchoResponse> {
    let mut buf = [0u8; 65535];

    loop {
        let (n, _) = transport.recv(&mut buf).await?;
        if n < HEADER_SIZE {
            continue;
        }

        let header = match Header::decode(&buf[..n]) {
            Ok(h) => h,
            Err(_) => continue,
        };

        if header.message_type != MessageType::EchoResponse {
            continue;
        }

        let response = EchoResponse::decode(&buf[HEADER_SIZE..n])?;
        if response.seq == expected_seq {
            return Ok(response);
        }
        // Out-of-order response; discard and keep waiting.
        debug!(
            expected = expected_seq,
            got = response.seq,
            "echo client: out-of-order response, discarding"
        );
    }
}

/// Get the current time in microseconds since the Unix epoch.
fn now_us() -> i64 {
    Utc::now().timestamp_micros()
}
