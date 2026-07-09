//! Video stream wire types for HyperLink Phase 2.
//!
//! Defines the binary framing for H.264 encoded video frames, codec
//! configuration records (SPS/PPS), and receiver-side bitrate feedback.
//! These are hot-path types — binary encoding, no JSON/serde overhead.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Cursor, Read, Write};

/// Header prepended to every encoded video frame on the wire.
///
/// Layout (25 bytes fixed):
/// ```text
///   frame_id      : u32  (4 bytes)
///   timestamp_us  : u64  (8 bytes)
///   flags         : u8   (1 byte, bit 0 = is_keyframe)
///   width         : u16  (2 bytes)
///   height        : u16  (2 bytes)
///   payload_len   : u32  (4 bytes)
///   fragment_idx  : u16  (2 bytes)
///   fragment_count: u16  (2 bytes)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrameHeader {
    /// Monotonically increasing frame counter.
    pub frame_id: u32,
    /// Capture timestamp in microseconds (from device monotonic clock).
    pub timestamp_us: u64,
    /// Whether this frame is an IDR/keyframe.
    pub is_keyframe: bool,
    /// Current capture width in pixels.
    pub width: u16,
    /// Current capture height in pixels.
    pub height: u16,
    /// Length of the NAL unit payload following this header.
    pub payload_len: u32,
    /// Fragment index (0-based) for frames split across multiple datagrams.
    pub fragment_idx: u16,
    /// Total number of fragments for this frame.
    pub fragment_count: u16,
}

/// Fixed size of the serialized `VideoFrameHeader`.
pub const VIDEO_FRAME_HEADER_SIZE: usize = 25;

impl VideoFrameHeader {
    /// Serialize this header into a byte buffer.
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u32::<BigEndian>(self.frame_id)?;
        buf.write_u64::<BigEndian>(self.timestamp_us)?;
        let flags: u8 = if self.is_keyframe { 1 } else { 0 };
        buf.write_u8(flags)?;
        buf.write_u16::<BigEndian>(self.width)?;
        buf.write_u16::<BigEndian>(self.height)?;
        buf.write_u32::<BigEndian>(self.payload_len)?;
        buf.write_u16::<BigEndian>(self.fragment_idx)?;
        buf.write_u16::<BigEndian>(self.fragment_count)?;
        Ok(())
    }

    /// Deserialize a header from a byte slice.
    pub fn decode(data: &[u8]) -> io::Result<Self> {
        if data.len() < VIDEO_FRAME_HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "video frame header too short: {} < {}",
                    data.len(),
                    VIDEO_FRAME_HEADER_SIZE
                ),
            ));
        }
        let mut cursor = Cursor::new(data);
        let frame_id = cursor.read_u32::<BigEndian>()?;
        let timestamp_us = cursor.read_u64::<BigEndian>()?;
        let flags = cursor.read_u8()?;
        let is_keyframe = (flags & 1) != 0;
        let width = cursor.read_u16::<BigEndian>()?;
        let height = cursor.read_u16::<BigEndian>()?;
        let payload_len = cursor.read_u32::<BigEndian>()?;
        let fragment_idx = cursor.read_u16::<BigEndian>()?;
        let fragment_count = cursor.read_u16::<BigEndian>()?;
        Ok(Self {
            frame_id,
            timestamp_us,
            is_keyframe,
            width,
            height,
            payload_len,
            fragment_idx,
            fragment_count,
        })
    }

    /// Encode header + NAL payload into a single datagram-ready buffer.
    pub fn encode_with_payload(&self, nal_data: &[u8]) -> io::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(VIDEO_FRAME_HEADER_SIZE + nal_data.len());
        self.encode(&mut buf)?;
        buf.write_all(nal_data)?;
        Ok(buf)
    }
}

/// Codec configuration sent once at stream start and on each keyframe.
///
/// Layout:
/// ```text
///   sps_len    : u16  (2 bytes)
///   sps        : [u8] (sps_len bytes)
///   pps_len    : u16  (2 bytes)
///   pps        : [u8] (pps_len bytes)
///   bitrate_bps: u32  (4 bytes)
///   fps        : u8   (1 byte)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoConfig {
    /// Sequence Parameter Set.
    pub sps: Vec<u8>,
    /// Picture Parameter Set.
    pub pps: Vec<u8>,
    /// Current encoder bitrate in bits per second.
    pub bitrate_bps: u32,
    /// Current frame rate.
    pub fps: u8,
}

impl VideoConfig {
    /// Serialize to bytes.
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u16::<BigEndian>(self.sps.len() as u16)?;
        buf.write_all(&self.sps)?;
        buf.write_u16::<BigEndian>(self.pps.len() as u16)?;
        buf.write_all(&self.pps)?;
        buf.write_u32::<BigEndian>(self.bitrate_bps)?;
        buf.write_u8(self.fps)?;
        Ok(())
    }

    /// Deserialize from bytes.
    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let sps_len = cursor.read_u16::<BigEndian>()? as usize;
        let mut sps = vec![0u8; sps_len];
        cursor.read_exact(&mut sps)?;
        let pps_len = cursor.read_u16::<BigEndian>()? as usize;
        let mut pps = vec![0u8; pps_len];
        cursor.read_exact(&mut pps)?;
        let bitrate_bps = cursor.read_u32::<BigEndian>()?;
        let fps = cursor.read_u8()?;
        Ok(Self {
            sps,
            pps,
            bitrate_bps,
            fps,
        })
    }

    /// Convenience: encode to a new Vec.
    pub fn to_bytes(&self) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.encode(&mut buf)?;
        Ok(buf)
    }
}

/// Receiver → sender bitrate feedback acknowledgement.
///
/// Layout (17 bytes):
/// ```text
///   timestamp_us     : u64 (8 bytes)
///   received_frames  : u32 (4 bytes)
///   lost_frames      : u32 (4 bytes)
///   suggested_bps    : u32 (4 bytes) — 0 means "no suggestion"
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitrateAck {
    /// Receiver timestamp when this ack was generated.
    pub timestamp_us: u64,
    /// Number of frames successfully received in the measurement window.
    pub received_frames: u32,
    /// Number of frames detected as lost (gap in frame_id sequence).
    pub lost_frames: u32,
    /// Suggested target bitrate in bits/sec, or 0 for no opinion.
    pub suggested_bps: u32,
}

/// Fixed size of serialized `BitrateAck`.
pub const BITRATE_ACK_SIZE: usize = 20;

impl BitrateAck {
    /// Serialize to bytes.
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u64::<BigEndian>(self.timestamp_us)?;
        buf.write_u32::<BigEndian>(self.received_frames)?;
        buf.write_u32::<BigEndian>(self.lost_frames)?;
        buf.write_u32::<BigEndian>(self.suggested_bps)?;
        Ok(())
    }

    /// Deserialize from bytes.
    pub fn decode(data: &[u8]) -> io::Result<Self> {
        if data.len() < BITRATE_ACK_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "bitrate ack too short: {} < {}",
                    data.len(),
                    BITRATE_ACK_SIZE
                ),
            ));
        }
        let mut cursor = Cursor::new(data);
        let timestamp_us = cursor.read_u64::<BigEndian>()?;
        let received_frames = cursor.read_u32::<BigEndian>()?;
        let lost_frames = cursor.read_u32::<BigEndian>()?;
        let suggested_bps = cursor.read_u32::<BigEndian>()?;
        Ok(Self {
            timestamp_us,
            received_frames,
            lost_frames,
            suggested_bps,
        })
    }
}

/// Checks if a received frame is "stale" compared to the newest frame we've seen.
///
/// A frame is stale if its frame_id is less than `newest_seen_id` — meaning
/// it arrived out of order after a newer frame was already decoded.
#[inline]
pub fn is_frame_stale(frame_id: u32, newest_seen_id: u32) -> bool {
    // Handle wrap-around: if the difference is huge, it's likely a wrap
    // and the frame is actually newer.
    let diff = newest_seen_id.wrapping_sub(frame_id);
    diff > 0 && diff < (u32::MAX / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_frame_header_round_trip() {
        let header = VideoFrameHeader {
            frame_id: 42,
            timestamp_us: 1_234_567_890,
            is_keyframe: true,
            width: 1920,
            height: 1080,
            payload_len: 65536,
            fragment_idx: 0,
            fragment_count: 1,
        };
        let mut buf = Vec::new();
        header.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), VIDEO_FRAME_HEADER_SIZE);

        let decoded = VideoFrameHeader::decode(&buf).unwrap();
        assert_eq!(header, decoded);
    }

    #[test]
    fn video_frame_header_non_keyframe() {
        let header = VideoFrameHeader {
            frame_id: 100,
            timestamp_us: 999_999,
            is_keyframe: false,
            width: 720,
            height: 1280,
            payload_len: 1024,
            fragment_idx: 2,
            fragment_count: 5,
        };
        let mut buf = Vec::new();
        header.encode(&mut buf).unwrap();
        let decoded = VideoFrameHeader::decode(&buf).unwrap();
        assert_eq!(header, decoded);
        assert!(!decoded.is_keyframe);
    }

    #[test]
    fn video_frame_header_with_payload() {
        let header = VideoFrameHeader {
            frame_id: 1,
            timestamp_us: 0,
            is_keyframe: true,
            width: 640,
            height: 480,
            payload_len: 4,
            fragment_idx: 0,
            fragment_count: 1,
        };
        let payload = b"\x00\x00\x00\x01"; // H.264 start code
        let datagram = header.encode_with_payload(payload).unwrap();
        assert_eq!(datagram.len(), VIDEO_FRAME_HEADER_SIZE + 4);

        let decoded_header = VideoFrameHeader::decode(&datagram).unwrap();
        assert_eq!(decoded_header.payload_len, 4);
        let decoded_payload = &datagram[VIDEO_FRAME_HEADER_SIZE..];
        assert_eq!(decoded_payload, payload);
    }

    #[test]
    fn video_frame_header_too_short() {
        let short = [0u8; 10];
        assert!(VideoFrameHeader::decode(&short).is_err());
    }

    #[test]
    fn video_config_round_trip() {
        let config = VideoConfig {
            sps: vec![0x67, 0x42, 0x00, 0x1e, 0xab, 0x40],
            pps: vec![0x68, 0xce, 0x38, 0x80],
            bitrate_bps: 4_000_000,
            fps: 30,
        };
        let bytes = config.to_bytes().unwrap();
        let decoded = VideoConfig::decode(&bytes).unwrap();
        assert_eq!(config, decoded);
    }

    #[test]
    fn video_config_empty_params() {
        let config = VideoConfig {
            sps: vec![],
            pps: vec![],
            bitrate_bps: 0,
            fps: 0,
        };
        let bytes = config.to_bytes().unwrap();
        let decoded = VideoConfig::decode(&bytes).unwrap();
        assert_eq!(config, decoded);
    }

    #[test]
    fn bitrate_ack_round_trip() {
        let ack = BitrateAck {
            timestamp_us: 1_000_000,
            received_frames: 300,
            lost_frames: 5,
            suggested_bps: 3_500_000,
        };
        let mut buf = Vec::new();
        ack.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), BITRATE_ACK_SIZE);

        let decoded = BitrateAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }

    #[test]
    fn bitrate_ack_no_suggestion() {
        let ack = BitrateAck {
            timestamp_us: 500_000,
            received_frames: 100,
            lost_frames: 0,
            suggested_bps: 0,
        };
        let mut buf = Vec::new();
        ack.encode(&mut buf).unwrap();
        let decoded = BitrateAck::decode(&buf).unwrap();
        assert_eq!(decoded.suggested_bps, 0);
    }

    #[test]
    fn bitrate_ack_too_short() {
        let short = [0u8; 10];
        assert!(BitrateAck::decode(&short).is_err());
    }

    #[test]
    fn frame_staleness_detection() {
        assert!(!is_frame_stale(5, 5)); // same frame, not stale
        assert!(is_frame_stale(4, 5)); // older frame, stale
        assert!(!is_frame_stale(6, 5)); // newer frame, not stale
        assert!(is_frame_stale(1, 100)); // much older, stale
    }

    #[test]
    fn frame_staleness_wraparound() {
        // When frame_id wraps around u32::MAX → 0, the new frame (0) should
        // not be considered stale relative to u32::MAX.
        assert!(!is_frame_stale(0, u32::MAX));
        // But u32::MAX - 1 is still stale relative to u32::MAX
        assert!(is_frame_stale(u32::MAX - 1, u32::MAX));
    }
}
