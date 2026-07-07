//! Message type definitions for the HyperLink protocol.
//!
//! Every wire message is tagged with a `MessageType` discriminant in the header.
//! Phase 0 defines clock-sync and echo types used by the bench harness;
//! subsequent phases extend this enum with video, input, control-plane, and
//! file-transfer types.

/// Discriminant for each message type on the wire.
///
/// Encoded as a single `u8` in the header. Values are assigned explicitly
/// (not auto-numbered) so wire compatibility survives reordering in source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MessageType {
    // --- Phase 0: Bench harness ---
    /// NTP-style clock synchronization request (client → server).
    ClockSyncRequest = 0x01,
    /// NTP-style clock synchronization response (server → client).
    ClockSyncResponse = 0x02,
    /// Echo request carrying a sequence number and timestamp (client → server).
    EchoRequest = 0x10,
    /// Echo response mirroring the request (server → client).
    EchoResponse = 0x11,
    /// Keepalive heartbeat (bidirectional).
    Heartbeat = 0x20,
    // --- Phase 1+: reserved ranges ---
    // 0x30–0x3F: video stream messages
    // 0x40–0x4F: input stream messages
    // 0x50–0x5F: control-plane messages (notifications, clipboard)
    // 0x60–0x6F: file stream messages
}

impl TryFrom<u8> for MessageType {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::ClockSyncRequest),
            0x02 => Ok(Self::ClockSyncResponse),
            0x10 => Ok(Self::EchoRequest),
            0x11 => Ok(Self::EchoResponse),
            0x20 => Ok(Self::Heartbeat),
            other => Err(other),
        }
    }
}

impl std::fmt::Display for MessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClockSyncRequest => write!(f, "ClockSyncRequest"),
            Self::ClockSyncResponse => write!(f, "ClockSyncResponse"),
            Self::EchoRequest => write!(f, "EchoRequest"),
            Self::EchoResponse => write!(f, "EchoResponse"),
            Self::Heartbeat => write!(f, "Heartbeat"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_types() {
        let types = [
            MessageType::ClockSyncRequest,
            MessageType::ClockSyncResponse,
            MessageType::EchoRequest,
            MessageType::EchoResponse,
            MessageType::Heartbeat,
        ];
        for msg_type in types {
            let raw = msg_type as u8;
            let decoded = MessageType::try_from(raw).unwrap();
            assert_eq!(msg_type, decoded);
        }
    }

    #[test]
    fn unknown_type_returns_err() {
        assert!(MessageType::try_from(0xFF).is_err());
    }
}
