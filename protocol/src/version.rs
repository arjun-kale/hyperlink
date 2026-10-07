//! Protocol versioning and wire header.
//!
//! Every HyperLink packet starts with a fixed header: magic bytes, protocol
//! version, message type, and payload length. The version byte is present from
//! day one so future protocol changes don't hard-break older builds.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Cursor, Read, Write};

use crate::message::MessageType;

/// Magic bytes identifying a HyperLink packet: `HLNK`.
pub const MAGIC: [u8; 4] = *b"HLNK";

/// Current protocol version. Incremented when wire-incompatible changes are made.
/// Introduced in Phase 0 so retrofitting is never needed.
pub const PROTOCOL_VERSION: u8 = 1;

/// Fixed-size header prepended to every HyperLink message on the wire.
///
/// Wire layout (10 bytes total):
/// ```text
/// ┌───────────┬─────────┬──────────────┬────────────────┐
/// │ magic (4) │ ver (1) │ msg_type (1) │ payload_len (4)│
/// └───────────┴─────────┴──────────────┴────────────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Protocol version byte.
    pub version: u8,
    /// Type of the message that follows.
    pub message_type: MessageType,
    /// Length of the payload in bytes (does not include the header itself).
    pub payload_len: u32,
}

/// Total size of the wire header in bytes.
pub const HEADER_SIZE: usize = 4 + 1 + 1 + 4; // magic + version + msg_type + payload_len

impl Header {
    /// Create a new header for a given message type and payload length.
    pub fn new(message_type: MessageType, payload_len: u32) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            message_type,
            payload_len,
        }
    }

    /// Encode this header into a byte buffer.
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_all(&MAGIC)?;
        buf.write_u8(self.version)?;
        buf.write_u8(self.message_type as u8)?;
        buf.write_u32::<BigEndian>(self.payload_len)?;
        Ok(())
    }

    /// Encode this header into a fixed-size byte array.
    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut buf = Vec::with_capacity(HEADER_SIZE);
        self.encode(&mut buf).expect("encoding to Vec cannot fail");
        let mut arr = [0u8; HEADER_SIZE];
        arr.copy_from_slice(&buf);
        arr
    }

    /// Decode a header from a byte slice.
    ///
    /// Returns an error if the magic bytes don't match or the slice is too short.
    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("header requires {} bytes, got {}", HEADER_SIZE, buf.len()),
            ));
        }

        let mut cursor = Cursor::new(buf);

        // Validate magic bytes.
        let mut magic = [0u8; 4];
        cursor.read_exact(&mut magic)?;
        if magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid magic: expected {:?}, got {:?}", MAGIC, magic),
            ));
        }

        let version = cursor.read_u8()?;
        let msg_type_raw = cursor.read_u8()?;
        let message_type = MessageType::try_from(msg_type_raw).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown message type: {}", msg_type_raw),
            )
        })?;
        let payload_len = cursor.read_u32::<BigEndian>()?;

        Ok(Self {
            version,
            message_type,
            payload_len,
        })
    }
}

/// Introduces one side of a session to the other. Sent first on the control
/// stream by both phone and computer, as JSON (it's tiny and sent once).
///
/// Peers that predate `Hello` never send one; receivers must treat a missing
/// Hello as "same protocol, name unknown" rather than an error.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Hello {
    /// The sender's `PROTOCOL_VERSION`.
    pub protocol: u8,
    /// The sender's app version, for display ("0.1.0").
    pub app_version: String,
    /// What the user calls this device ("Galaxy A15 5G", "Arjun's Laptop").
    pub name: String,
    /// "android" or "linux".
    pub platform: String,
}

impl Hello {
    pub fn new(app_version: &str, name: &str, platform: &str) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            app_version: app_version.to_string(),
            name: name.to_string(),
            platform: platform.to_string(),
        }
    }

    /// Whether the peer speaks the same protocol as this build.
    pub fn is_compatible(&self) -> bool {
        self.protocol == PROTOCOL_VERSION
    }

    /// Header + JSON payload, ready to write to the control stream.
    pub fn to_packet(&self) -> Vec<u8> {
        let payload = serde_json::to_vec(self).unwrap_or_default();
        let mut packet = Vec::with_capacity(HEADER_SIZE + payload.len());
        let _ = Header::new(MessageType::Hello, payload.len() as u32).encode(&mut packet);
        packet.extend_from_slice(&payload);
        packet
    }

    pub fn decode(payload: &[u8]) -> Option<Self> {
        serde_json::from_slice(payload).ok()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hello_round_trips_and_checks_version() {
        let hello = super::Hello::new("0.1.0", "Galaxy A15 5G", "android");
        let packet = hello.to_packet();
        let header = super::Header::decode(&packet).unwrap();
        assert_eq!(header.message_type, crate::message::MessageType::Hello);
        let decoded = super::Hello::decode(&packet[super::HEADER_SIZE..]).unwrap();
        assert_eq!(decoded, hello);
        assert!(decoded.is_compatible());
        let future = super::Hello {
            protocol: super::PROTOCOL_VERSION + 1,
            ..decoded
        };
        assert!(!future.is_compatible());
    }

    use super::*;

    #[test]
    fn header_round_trip() {
        let header = Header::new(MessageType::EchoRequest, 42);
        let bytes = header.to_bytes();
        let decoded = Header::decode(&bytes).unwrap();
        assert_eq!(header, decoded);
    }

    #[test]
    fn header_magic_check() {
        let mut bytes = Header::new(MessageType::Heartbeat, 0).to_bytes();
        bytes[0] = b'X'; // corrupt magic
        assert!(Header::decode(&bytes).is_err());
    }

    #[test]
    fn header_too_short() {
        let bytes = [0u8; 5]; // less than HEADER_SIZE
        assert!(Header::decode(&bytes).is_err());
    }

    #[test]
    fn header_size_is_10() {
        assert_eq!(HEADER_SIZE, 10);
    }
}
