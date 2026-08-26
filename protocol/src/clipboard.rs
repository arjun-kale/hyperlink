//! Clipboard synchronization protocol wire types (Phase 5).
//!
//! Defines messages for bidirectional text and image clipboard sharing,
//! origin tracking for ping-pong loop prevention, and delivery acknowledgements.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{self, Cursor, Read};

/// Maximum allowed length for origin ID and MIME type strings (prevent DoS).
pub const MAX_STRING_LEN: usize = 256;

/// Maximum allowed payload size for clipboard items (16 MB).
pub const MAX_CLIPBOARD_PAYLOAD: usize = 16 * 1024 * 1024;

/// Computes SHA-256 hash of clipboard payload for content-based deduplication.
pub fn compute_content_hash(payload: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(payload);
    hasher.finalize().into()
}

/// Clipboard payload content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ClipboardType {
    Text = 0x01,
    Image = 0x02,
}

impl TryFrom<u8> for ClipboardType {
    type Error = io::Error;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0x01 => Ok(Self::Text),
            0x02 => Ok(Self::Image),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown clipboard type: {v:#04x}"),
            )),
        }
    }
}

/// Clipboard item synced over the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardMessage {
    /// Origin identifier (e.g. "host" or "phone:<device_id>") to break echo loops.
    pub origin_id: String,
    /// Content classification (Text or Image).
    pub content_type: ClipboardType,
    /// MIME type string (e.g. "text/plain", "text/html", "image/png").
    pub mime_type: String,
    /// SHA-256 hash of payload for content deduplication.
    pub content_hash: [u8; 32],
    /// Monotonic sequence number from origin.
    pub seq: u64,
    /// Origin capture timestamp in microseconds.
    pub timestamp_us: u64,
    /// Raw payload bytes (UTF-8 text or compressed image data).
    pub payload: Vec<u8>,
}

impl ClipboardMessage {
    /// Creates a text clipboard message.
    pub fn new_text(origin_id: String, text: &str, seq: u64, timestamp_us: u64) -> Self {
        let payload = text.as_bytes().to_vec();
        let content_hash = compute_content_hash(&payload);
        Self {
            origin_id,
            content_type: ClipboardType::Text,
            mime_type: "text/plain".to_string(),
            content_hash,
            seq,
            timestamp_us,
            payload,
        }
    }

    /// Creates an image clipboard message.
    pub fn new_image(
        origin_id: String,
        mime_type: String,
        image_bytes: Vec<u8>,
        seq: u64,
        timestamp_us: u64,
    ) -> Self {
        let content_hash = compute_content_hash(&image_bytes);
        Self {
            origin_id,
            content_type: ClipboardType::Image,
            mime_type,
            content_hash,
            seq,
            timestamp_us,
            payload: image_bytes,
        }
    }

    /// Encodes clipboard message into byte buffer.
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        if self.payload.len() > MAX_CLIPBOARD_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("clipboard payload exceeds maximum size of {MAX_CLIPBOARD_PAYLOAD} bytes"),
            ));
        }

        write_string(dst, &self.origin_id)?;
        dst.write_u8(self.content_type as u8)?;
        write_string(dst, &self.mime_type)?;
        dst.extend_from_slice(&self.content_hash);
        dst.write_u64::<BigEndian>(self.seq)?;
        dst.write_u64::<BigEndian>(self.timestamp_us)?;
        dst.write_u32::<BigEndian>(self.payload.len() as u32)?;
        dst.extend_from_slice(&self.payload);
        Ok(())
    }

    /// Decodes clipboard message from byte buffer.
    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let origin_id = read_string(&mut cursor)?;
        let content_type = ClipboardType::try_from(cursor.read_u8()?)?;
        let mime_type = read_string(&mut cursor)?;

        let mut content_hash = [0u8; 32];
        cursor.read_exact(&mut content_hash)?;

        let seq = cursor.read_u64::<BigEndian>()?;
        let timestamp_us = cursor.read_u64::<BigEndian>()?;

        let payload_len = cursor.read_u32::<BigEndian>()? as usize;
        if payload_len > MAX_CLIPBOARD_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("payload length {payload_len} exceeds maximum allowed"),
            ));
        }

        let mut payload = vec![0u8; payload_len];
        cursor.read_exact(&mut payload)?;

        Ok(Self {
            origin_id,
            content_type,
            mime_type,
            content_hash,
            seq,
            timestamp_us,
            payload,
        })
    }
}

/// Clipboard delivery acknowledgment for round-trip latency accounting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardAck {
    pub seq: u64,
    pub content_hash: [u8; 32],
    pub origin_id: String,
    pub host_received_us: u64,
}

impl ClipboardAck {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.seq)?;
        dst.extend_from_slice(&self.content_hash);
        write_string(dst, &self.origin_id)?;
        dst.write_u64::<BigEndian>(self.host_received_us)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let seq = cursor.read_u64::<BigEndian>()?;
        let mut content_hash = [0u8; 32];
        cursor.read_exact(&mut content_hash)?;
        let origin_id = read_string(&mut cursor)?;
        let host_received_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            seq,
            content_hash,
            origin_id,
            host_received_us,
        })
    }
}

fn write_string(dst: &mut Vec<u8>, s: &str) -> io::Result<()> {
    let bytes = s.as_bytes();
    if bytes.len() > MAX_STRING_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "string too long",
        ));
    }
    dst.write_u16::<BigEndian>(bytes.len() as u16)?;
    dst.extend_from_slice(bytes);
    Ok(())
}

fn read_string(cursor: &mut Cursor<&[u8]>) -> io::Result<String> {
    let len = cursor.read_u16::<BigEndian>()? as usize;
    if len > MAX_STRING_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "string too long",
        ));
    }
    let mut bytes = vec![0u8; len];
    cursor.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_clipboard_round_trip() {
        let msg = ClipboardMessage::new_text(
            "host:linux-workstation".to_string(),
            "Hello from Linux clipboard!",
            1,
            1_000_000,
        );
        let mut buf = Vec::new();
        msg.encode(&mut buf).unwrap();
        let decoded = ClipboardMessage::decode(&buf).unwrap();
        assert_eq!(msg, decoded);
        assert_eq!(decoded.content_type, ClipboardType::Text);
        assert_eq!(
            String::from_utf8(decoded.payload).unwrap(),
            "Hello from Linux clipboard!"
        );
    }

    #[test]
    fn test_image_clipboard_round_trip() {
        let fake_image = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0xFF];
        let msg = ClipboardMessage::new_image(
            "phone:pixel8".to_string(),
            "image/png".to_string(),
            fake_image.clone(),
            42,
            2_000_000,
        );
        let mut buf = Vec::new();
        msg.encode(&mut buf).unwrap();
        let decoded = ClipboardMessage::decode(&buf).unwrap();
        assert_eq!(msg, decoded);
        assert_eq!(decoded.content_type, ClipboardType::Image);
        assert_eq!(decoded.payload, fake_image);
    }

    #[test]
    fn test_content_hash_consistency() {
        let text1 = b"Hello, World!";
        let text2 = b"Hello, World!";
        let text3 = b"Different text";
        assert_eq!(compute_content_hash(text1), compute_content_hash(text2));
        assert_ne!(compute_content_hash(text1), compute_content_hash(text3));
    }

    #[test]
    fn test_clipboard_ack_round_trip() {
        let ack = ClipboardAck {
            seq: 100,
            content_hash: compute_content_hash(b"test"),
            origin_id: "host".to_string(),
            host_received_us: 3_456_789,
        };
        let mut buf = Vec::new();
        ack.encode(&mut buf).unwrap();
        let decoded = ClipboardAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn test_oversize_payload_rejected() {
        let oversize = vec![0u8; MAX_CLIPBOARD_PAYLOAD + 1];
        let msg = ClipboardMessage {
            origin_id: "host".to_string(),
            content_type: ClipboardType::Image,
            mime_type: "image/png".to_string(),
            content_hash: [0u8; 32],
            seq: 1,
            timestamp_us: 0,
            payload: oversize,
        };
        let mut buf = Vec::new();
        assert!(msg.encode(&mut buf).is_err());
    }
}
