//! Echo request/response payloads for round-trip latency measurement.
//!
//! The echo tool is the simplest latency probe: send a timestamped packet,
//! get it back, measure the round trip. This works even without clock sync
//! (RTT doesn't need synchronized clocks), and is used alongside clock sync
//! to estimate one-way latency.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{self, Cursor};

/// Echo request sent from client to server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchoRequest {
    /// Sequence number to correlate request/response.
    pub seq: u32,
    /// Client send timestamp in microseconds since epoch.
    pub send_time_us: i64,
    /// Optional payload bytes (for measuring throughput under varying sizes).
    /// Not serialized on the wire — the raw padding bytes follow the fixed fields.
    pub payload_size: u32,
}

/// Echo response sent from server to client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchoResponse {
    /// Sequence number echoed from the request.
    pub seq: u32,
    /// Client send timestamp echoed from the request.
    pub send_time_us: i64,
    /// Server receive timestamp in microseconds since epoch.
    pub server_recv_time_us: i64,
    /// Payload size echoed from the request.
    pub payload_size: u32,
}

/// Wire size of the fixed fields in an `EchoRequest` (excluding variable payload).
pub const ECHO_REQUEST_FIXED_SIZE: usize = 4 + 8 + 4; // seq(4) + send_time(8) + payload_size(4)

/// Wire size of the fixed fields in an `EchoResponse`.
pub const ECHO_RESPONSE_FIXED_SIZE: usize = 4 + 8 + 8 + 4; // seq(4) + send_time(8) + server_recv(8) + payload_size(4)

impl EchoRequest {
    /// Encode the fixed fields into a byte buffer.
    /// Caller should append `payload_size` bytes of padding after this if desired.
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u32::<BigEndian>(self.seq)?;
        buf.write_i64::<BigEndian>(self.send_time_us)?;
        buf.write_u32::<BigEndian>(self.payload_size)?;
        Ok(())
    }

    /// Encode to a byte vector (fixed fields only, no padding).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(ECHO_REQUEST_FIXED_SIZE);
        self.encode(&mut buf).expect("encoding to Vec cannot fail");
        buf
    }

    /// Decode from a byte slice.
    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < ECHO_REQUEST_FIXED_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "echo request too short",
            ));
        }
        let mut cursor = Cursor::new(buf);
        let seq = cursor.read_u32::<BigEndian>()?;
        let send_time_us = cursor.read_i64::<BigEndian>()?;
        let payload_size = cursor.read_u32::<BigEndian>()?;
        Ok(Self {
            seq,
            send_time_us,
            payload_size,
        })
    }
}

impl EchoResponse {
    /// Encode into a byte buffer.
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u32::<BigEndian>(self.seq)?;
        buf.write_i64::<BigEndian>(self.send_time_us)?;
        buf.write_i64::<BigEndian>(self.server_recv_time_us)?;
        buf.write_u32::<BigEndian>(self.payload_size)?;
        Ok(())
    }

    /// Encode to a byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(ECHO_RESPONSE_FIXED_SIZE);
        self.encode(&mut buf).expect("encoding to Vec cannot fail");
        buf
    }

    /// Decode from a byte slice.
    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < ECHO_RESPONSE_FIXED_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "echo response too short",
            ));
        }
        let mut cursor = Cursor::new(buf);
        let seq = cursor.read_u32::<BigEndian>()?;
        let send_time_us = cursor.read_i64::<BigEndian>()?;
        let server_recv_time_us = cursor.read_i64::<BigEndian>()?;
        let payload_size = cursor.read_u32::<BigEndian>()?;
        Ok(Self {
            seq,
            send_time_us,
            server_recv_time_us,
            payload_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_request_round_trip() {
        let req = EchoRequest {
            seq: 1,
            send_time_us: 123_456_789,
            payload_size: 64,
        };
        let bytes = req.to_bytes();
        assert_eq!(bytes.len(), ECHO_REQUEST_FIXED_SIZE);
        let decoded = EchoRequest::decode(&bytes).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn echo_response_round_trip() {
        let resp = EchoResponse {
            seq: 1,
            send_time_us: 123_456_789,
            server_recv_time_us: 123_456_800,
            payload_size: 64,
        };
        let bytes = resp.to_bytes();
        assert_eq!(bytes.len(), ECHO_RESPONSE_FIXED_SIZE);
        let decoded = EchoResponse::decode(&bytes).unwrap();
        assert_eq!(resp, decoded);
    }
}
