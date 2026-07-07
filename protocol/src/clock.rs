//! NTP-style clock synchronization payloads.
//!
//! Cross-device latency measurement is meaningless without synchronized clocks.
//! This module implements an NTP-style four-timestamp exchange to estimate the
//! clock offset between the bench client and server.
//!
//! ## Algorithm
//!
//! ```text
//! Client                    Server
//!   |--- ClockSyncRequest --->|
//!   |    t1 = client_send     |   t2 = server_recv
//!   |                         |   t3 = server_send
//!   |<-- ClockSyncResponse ---|
//!   |    t4 = client_recv     |
//! ```
//!
//! - **Offset** = `((t2 - t1) + (t3 - t4)) / 2`
//! - **Round-trip delay** = `(t4 - t1) - (t3 - t2)`

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{self, Cursor};

/// Clock synchronization request sent from client to server.
///
/// Contains the client's send timestamp (`t1`) and a sequence number
/// to correlate request/response pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockSyncRequest {
    /// Sequence number for correlating request/response pairs.
    pub seq: u32,
    /// Client send timestamp in microseconds since epoch (`t1`).
    pub t1_us: i64,
}

/// Clock synchronization response sent from server to client.
///
/// Contains all four NTP-style timestamps needed to compute offset and delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockSyncResponse {
    /// Sequence number echoed from the request.
    pub seq: u32,
    /// Client send timestamp in microseconds since epoch (`t1`), echoed.
    pub t1_us: i64,
    /// Server receive timestamp in microseconds since epoch (`t2`).
    pub t2_us: i64,
    /// Server send timestamp in microseconds since epoch (`t3`).
    pub t3_us: i64,
}

/// Wire size of a `ClockSyncRequest` payload in bytes.
pub const CLOCK_SYNC_REQUEST_SIZE: usize = 4 + 8; // seq(4) + t1(8)

/// Wire size of a `ClockSyncResponse` payload in bytes.
pub const CLOCK_SYNC_RESPONSE_SIZE: usize = 4 + 8 + 8 + 8; // seq(4) + t1(8) + t2(8) + t3(8)

impl ClockSyncRequest {
    /// Encode this request into a byte buffer.
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u32::<BigEndian>(self.seq)?;
        buf.write_i64::<BigEndian>(self.t1_us)?;
        Ok(())
    }

    /// Encode to a byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(CLOCK_SYNC_REQUEST_SIZE);
        self.encode(&mut buf).expect("encoding to Vec cannot fail");
        buf
    }

    /// Decode from a byte slice.
    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < CLOCK_SYNC_REQUEST_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "clock sync request too short",
            ));
        }
        let mut cursor = Cursor::new(buf);
        let seq = cursor.read_u32::<BigEndian>()?;
        let t1_us = cursor.read_i64::<BigEndian>()?;
        Ok(Self { seq, t1_us })
    }
}

impl ClockSyncResponse {
    /// Encode this response into a byte buffer.
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u32::<BigEndian>(self.seq)?;
        buf.write_i64::<BigEndian>(self.t1_us)?;
        buf.write_i64::<BigEndian>(self.t2_us)?;
        buf.write_i64::<BigEndian>(self.t3_us)?;
        Ok(())
    }

    /// Encode to a byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(CLOCK_SYNC_RESPONSE_SIZE);
        self.encode(&mut buf).expect("encoding to Vec cannot fail");
        buf
    }

    /// Decode from a byte slice.
    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < CLOCK_SYNC_RESPONSE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "clock sync response too short",
            ));
        }
        let mut cursor = Cursor::new(buf);
        let seq = cursor.read_u32::<BigEndian>()?;
        let t1_us = cursor.read_i64::<BigEndian>()?;
        let t2_us = cursor.read_i64::<BigEndian>()?;
        let t3_us = cursor.read_i64::<BigEndian>()?;
        Ok(Self {
            seq,
            t1_us,
            t2_us,
            t3_us,
        })
    }

    /// Compute the estimated clock offset in microseconds.
    ///
    /// `offset = ((t2 - t1) + (t3 - t4)) / 2`
    ///
    /// A positive offset means the server clock is ahead of the client.
    pub fn compute_offset(&self, t4_us: i64) -> i64 {
        ((self.t2_us - self.t1_us) + (self.t3_us - t4_us)) / 2
    }

    /// Compute the round-trip delay in microseconds.
    ///
    /// `delay = (t4 - t1) - (t3 - t2)`
    pub fn compute_delay(&self, t4_us: i64) -> i64 {
        (t4_us - self.t1_us) - (self.t3_us - self.t2_us)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip() {
        let req = ClockSyncRequest {
            seq: 42,
            t1_us: 1_000_000,
        };
        let bytes = req.to_bytes();
        assert_eq!(bytes.len(), CLOCK_SYNC_REQUEST_SIZE);
        let decoded = ClockSyncRequest::decode(&bytes).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn response_round_trip() {
        let resp = ClockSyncResponse {
            seq: 7,
            t1_us: 1_000_000,
            t2_us: 1_000_100,
            t3_us: 1_000_150,
        };
        let bytes = resp.to_bytes();
        assert_eq!(bytes.len(), CLOCK_SYNC_RESPONSE_SIZE);
        let decoded = ClockSyncResponse::decode(&bytes).unwrap();
        assert_eq!(resp, decoded);
    }

    #[test]
    fn offset_and_delay_calculation() {
        // Simulate: server clock is 50µs ahead.
        // t1=1000, t2=1150 (server: 1000+100 network + 50 offset),
        // t3=1160 (server processes for 10µs), t4=1260 (100µs return trip)
        let resp = ClockSyncResponse {
            seq: 1,
            t1_us: 1000,
            t2_us: 1150,
            t3_us: 1160,
        };
        let t4_us = 1260;

        let offset = resp.compute_offset(t4_us);
        // ((1150-1000) + (1160-1260)) / 2 = (150 + (-100)) / 2 = 25
        assert_eq!(offset, 25);

        let delay = resp.compute_delay(t4_us);
        // (1260-1000) - (1160-1150) = 260 - 10 = 250
        assert_eq!(delay, 250);
    }
}
