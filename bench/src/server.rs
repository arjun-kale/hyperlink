//! Unified server dispatcher.
//!
//! A single recv loop that dispatches incoming packets to the appropriate
//! handler based on message type. This avoids two handlers competing for
//! the same socket.

use std::net::SocketAddr;

use chrono::Utc;
use tracing::{debug, info, warn};

use hyperlink_protocol::clock::{ClockSyncRequest, ClockSyncResponse, CLOCK_SYNC_RESPONSE_SIZE};
use hyperlink_protocol::echo::{EchoRequest, EchoResponse, ECHO_RESPONSE_FIXED_SIZE};
use hyperlink_protocol::message::MessageType;
use hyperlink_protocol::version::{Header, HEADER_SIZE};

use crate::transport::{Transport, UdpTransport};

/// Run the unified bench server — handles all message types in a single recv loop.
pub async fn run(transport: &UdpTransport) -> std::io::Result<()> {
    let mut buf = [0u8; 65535];
    info!("server: listening for clock sync and echo requests");

    loop {
        let (n, peer) = transport.recv(&mut buf).await?;
        if n < HEADER_SIZE {
            debug!(bytes = n, "server: packet too short, ignoring");
            continue;
        }

        let header = match Header::decode(&buf[..n]) {
            Ok(h) => h,
            Err(e) => {
                debug!("server: invalid header: {e}");
                continue;
            }
        };

        let payload = &buf[HEADER_SIZE..n];

        match header.message_type {
            MessageType::ClockSyncRequest => {
                handle_clock_sync(transport, payload, peer).await?;
            }
            MessageType::EchoRequest => {
                handle_echo(transport, payload, peer).await?;
            }
            other => {
                debug!(msg_type = %other, "server: unhandled message type");
            }
        }
    }
}

async fn handle_clock_sync(
    transport: &UdpTransport,
    payload: &[u8],
    peer: SocketAddr,
) -> std::io::Result<()> {
    let request = match ClockSyncRequest::decode(payload) {
        Ok(r) => r,
        Err(e) => {
            warn!("server: failed to decode clock sync request: {e}");
            return Ok(());
        }
    };

    let t2_us = Utc::now().timestamp_micros();

    let response = ClockSyncResponse {
        seq: request.seq,
        t1_us: request.t1_us,
        t2_us,
        t3_us: Utc::now().timestamp_micros(),
    };

    let mut resp_buf = Vec::with_capacity(HEADER_SIZE + CLOCK_SYNC_RESPONSE_SIZE);
    let resp_header = Header::new(
        MessageType::ClockSyncResponse,
        CLOCK_SYNC_RESPONSE_SIZE as u32,
    );
    resp_header.encode(&mut resp_buf)?;
    response.encode(&mut resp_buf)?;

    transport.send_to(&resp_buf, peer).await?;
    debug!(seq = request.seq, "server: clock sync response sent");
    Ok(())
}

async fn handle_echo(
    transport: &UdpTransport,
    payload: &[u8],
    peer: SocketAddr,
) -> std::io::Result<()> {
    let request = match EchoRequest::decode(payload) {
        Ok(r) => r,
        Err(e) => {
            warn!("server: failed to decode echo request: {e}");
            return Ok(());
        }
    };

    let server_recv_time_us = Utc::now().timestamp_micros();

    let response = EchoResponse {
        seq: request.seq,
        send_time_us: request.send_time_us,
        server_recv_time_us,
        payload_size: request.payload_size,
    };

    let mut resp_buf = Vec::with_capacity(HEADER_SIZE + ECHO_RESPONSE_FIXED_SIZE);
    let resp_header = Header::new(MessageType::EchoResponse, ECHO_RESPONSE_FIXED_SIZE as u32);
    resp_header.encode(&mut resp_buf)?;
    response.encode(&mut resp_buf)?;

    transport.send_to(&resp_buf, peer).await?;
    debug!(seq = request.seq, "server: echo response sent");
    Ok(())
}
