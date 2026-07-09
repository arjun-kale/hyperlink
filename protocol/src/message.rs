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
    // --- Phase 2: video stream messages (0x30–0x3F) ---
    /// Encoded H.264 video frame (phone → host, unreliable datagram).
    VideoFrame = 0x30,
    /// Codec configuration: SPS/PPS + encoder params (phone → host, reliable).
    VideoConfig = 0x31,
    /// Receiver bitrate feedback (host → phone, reliable).
    BitrateAck = 0x32,
    // 0x40–0x4F: input stream messages (Phase 3)
    // 0x50–0x5F: control-plane messages (notifications, clipboard)
    /// Client initiates pairing and sends its name (client → server).
    PairRequest = 0x50,
    /// Server accepts or rejects pairing (server → client).
    PairResponse = 0x51,
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
            0x30 => Ok(Self::VideoFrame),
            0x31 => Ok(Self::VideoConfig),
            0x32 => Ok(Self::BitrateAck),
            0x50 => Ok(Self::PairRequest),
            0x51 => Ok(Self::PairResponse),
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
            Self::VideoFrame => write!(f, "VideoFrame"),
            Self::VideoConfig => write!(f, "VideoConfig"),
            Self::BitrateAck => write!(f, "BitrateAck"),
            Self::PairRequest => write!(f, "PairRequest"),
            Self::PairResponse => write!(f, "PairResponse"),
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
            MessageType::VideoFrame,
            MessageType::VideoConfig,
            MessageType::BitrateAck,
            MessageType::PairRequest,
            MessageType::PairResponse,
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
