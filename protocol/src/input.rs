//! Input stream wire types for HyperLink Phase 3.
//!
//! Defines the binary wire format for pointer (mouse/touch), keyboard,
//! scroll, navigation actions, and input round-trip acknowledgements.
//! These are latency-critical wire messages using compact binary encoding.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Cursor};

/// Pointer action discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PointerAction {
    Down = 0x01,
    Up = 0x02,
    Move = 0x03,
    Cancel = 0x04,
}

impl TryFrom<u8> for PointerAction {
    type Error = io::Error;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0x01 => Ok(Self::Down),
            0x02 => Ok(Self::Up),
            0x03 => Ok(Self::Move),
            0x04 => Ok(Self::Cancel),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown pointer action: {v}"),
            )),
        }
    }
}

/// Pointer button discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PointerButton {
    None = 0x00,
    Primary = 0x01,   // Left click or primary touch
    Secondary = 0x02, // Right click
    Middle = 0x03,
}

impl TryFrom<u8> for PointerButton {
    type Error = io::Error;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0x00 => Ok(Self::None),
            0x01 => Ok(Self::Primary),
            0x02 => Ok(Self::Secondary),
            0x03 => Ok(Self::Middle),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown pointer button: {v}"),
            )),
        }
    }
}

/// Pointer event message (15 bytes fixed).
///
/// Normalized coordinates use fixed-point `0..65535` representing `0.0..1.0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerEvent {
    pub action: PointerAction,
    pub button: PointerButton,
    pub x_norm: u16,
    pub y_norm: u16,
    pub pressure: u8,
    pub timestamp_us: u64,
}

pub const POINTER_EVENT_SIZE: usize = 1 + 1 + 2 + 2 + 1 + 8; // 15 bytes

impl PointerEvent {
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u8(self.action as u8)?;
        buf.write_u8(self.button as u8)?;
        buf.write_u16::<BigEndian>(self.x_norm)?;
        buf.write_u16::<BigEndian>(self.y_norm)?;
        buf.write_u8(self.pressure)?;
        buf.write_u64::<BigEndian>(self.timestamp_us)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        if data.len() < POINTER_EVENT_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "pointer event too short: {} < {POINTER_EVENT_SIZE}",
                    data.len()
                ),
            ));
        }
        let mut cursor = Cursor::new(data);
        let action = PointerAction::try_from(cursor.read_u8()?)?;
        let button = PointerButton::try_from(cursor.read_u8()?)?;
        let x_norm = cursor.read_u16::<BigEndian>()?;
        let y_norm = cursor.read_u16::<BigEndian>()?;
        let pressure = cursor.read_u8()?;
        let timestamp_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            action,
            button,
            x_norm,
            y_norm,
            pressure,
            timestamp_us,
        })
    }

    /// Convenience: convert float `(0.0..=1.0)` to normalized fixed-point `u16`.
    pub fn float_to_norm(val: f64) -> u16 {
        (val.clamp(0.0, 1.0) * 65535.0).round() as u16
    }

    /// Convenience: convert normalized fixed-point `u16` back to float `0.0..=1.0`.
    pub fn norm_to_float(val: u16) -> f64 {
        val as f64 / 65535.0
    }
}

/// Keyboard key action discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum KeyAction {
    Down = 0x01,
    Up = 0x02,
}

impl TryFrom<u8> for KeyAction {
    type Error = io::Error;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0x01 => Ok(Self::Down),
            0x02 => Ok(Self::Up),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown key action: {v}"),
            )),
        }
    }
}

/// Modifier keys bitflags.
pub mod key_modifiers {
    pub const SHIFT: u8 = 1 << 0;
    pub const CTRL: u8 = 1 << 1;
    pub const ALT: u8 = 1 << 2;
    pub const META: u8 = 1 << 3;
}

/// Key event message (14 bytes fixed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub action: KeyAction,
    pub keycode: u32,
    pub modifiers: u8,
    pub timestamp_us: u64,
}

pub const KEY_EVENT_SIZE: usize = 1 + 4 + 1 + 8; // 14 bytes

impl KeyEvent {
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u8(self.action as u8)?;
        buf.write_u32::<BigEndian>(self.keycode)?;
        buf.write_u8(self.modifiers)?;
        buf.write_u64::<BigEndian>(self.timestamp_us)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        if data.len() < KEY_EVENT_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("key event too short: {} < {KEY_EVENT_SIZE}", data.len()),
            ));
        }
        let mut cursor = Cursor::new(data);
        let action = KeyAction::try_from(cursor.read_u8()?)?;
        let keycode = cursor.read_u32::<BigEndian>()?;
        let modifiers = cursor.read_u8()?;
        let timestamp_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            action,
            keycode,
            modifiers,
            timestamp_us,
        })
    }
}

/// Scroll wheel delta event (16 bytes fixed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollEvent {
    pub dx: i16,
    pub dy: i16,
    pub x_norm: u16,
    pub y_norm: u16,
    pub timestamp_us: u64,
}

pub const SCROLL_EVENT_SIZE: usize = 2 + 2 + 2 + 2 + 8; // 16 bytes

impl ScrollEvent {
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_i16::<BigEndian>(self.dx)?;
        buf.write_i16::<BigEndian>(self.dy)?;
        buf.write_u16::<BigEndian>(self.x_norm)?;
        buf.write_u16::<BigEndian>(self.y_norm)?;
        buf.write_u64::<BigEndian>(self.timestamp_us)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        if data.len() < SCROLL_EVENT_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "scroll event too short: {} < {SCROLL_EVENT_SIZE}",
                    data.len()
                ),
            ));
        }
        let mut cursor = Cursor::new(data);
        let dx = cursor.read_i16::<BigEndian>()?;
        let dy = cursor.read_i16::<BigEndian>()?;
        let x_norm = cursor.read_u16::<BigEndian>()?;
        let y_norm = cursor.read_u16::<BigEndian>()?;
        let timestamp_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            dx,
            dy,
            x_norm,
            y_norm,
            timestamp_us,
        })
    }
}

/// System navigation action discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NavAction {
    Back = 0x01,
    Home = 0x02,
    Recents = 0x03,
    VolumeUp = 0x04,
    VolumeDown = 0x05,
}

impl TryFrom<u8> for NavAction {
    type Error = io::Error;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0x01 => Ok(Self::Back),
            0x02 => Ok(Self::Home),
            0x03 => Ok(Self::Recents),
            0x04 => Ok(Self::VolumeUp),
            0x05 => Ok(Self::VolumeDown),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown nav action: {v}"),
            )),
        }
    }
}

/// System navigation event message (9 bytes fixed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavEvent {
    pub action: NavAction,
    pub timestamp_us: u64,
}

pub const NAV_EVENT_SIZE: usize = 1 + 8; // 9 bytes

impl NavEvent {
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u8(self.action as u8)?;
        buf.write_u64::<BigEndian>(self.timestamp_us)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        if data.len() < NAV_EVENT_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("nav event too short: {} < {NAV_EVENT_SIZE}", data.len()),
            ));
        }
        let mut cursor = Cursor::new(data);
        let action = NavAction::try_from(cursor.read_u8()?)?;
        let timestamp_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            action,
            timestamp_us,
        })
    }
}

/// Input acknowledgement message for RTT and delivery validation (12 bytes fixed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputAck {
    pub seq: u32,
    pub timestamp_us: u64,
}

pub const INPUT_ACK_SIZE: usize = 4 + 8; // 12 bytes

impl InputAck {
    pub fn encode(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.write_u32::<BigEndian>(self.seq)?;
        buf.write_u64::<BigEndian>(self.timestamp_us)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        if data.len() < INPUT_ACK_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("input ack too short: {} < {INPUT_ACK_SIZE}", data.len()),
            ));
        }
        let mut cursor = Cursor::new(data);
        let seq = cursor.read_u32::<BigEndian>()?;
        let timestamp_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self { seq, timestamp_us })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pointer_event_round_trip() {
        let event = PointerEvent {
            action: PointerAction::Move,
            button: PointerButton::Primary,
            x_norm: PointerEvent::float_to_norm(0.75),
            y_norm: PointerEvent::float_to_norm(0.25),
            pressure: 128,
            timestamp_us: 1_234_567_890,
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), POINTER_EVENT_SIZE);
        let decoded = PointerEvent::decode(&buf).unwrap();
        assert_eq!(event, decoded);
        assert!((PointerEvent::norm_to_float(decoded.x_norm) - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_key_event_round_trip() {
        let event = KeyEvent {
            action: KeyAction::Down,
            keycode: 65, // Key A
            modifiers: key_modifiers::SHIFT | key_modifiers::CTRL,
            timestamp_us: 987_654_321,
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), KEY_EVENT_SIZE);
        let decoded = KeyEvent::decode(&buf).unwrap();
        assert_eq!(event, decoded);
    }

    #[test]
    fn test_key_event_keysym_ascii_values() {
        let letters = [
            ('a', 97),
            ('A', 65),
            ('z', 122),
            ('Z', 90),
            (' ', 32),
            ('\n', 10),
        ];
        for (ch, expected_code) in letters {
            let keycode = ch as u32;
            assert_eq!(keycode, expected_code);
            let event = KeyEvent {
                action: KeyAction::Down,
                keycode,
                modifiers: 0,
                timestamp_us: 1000,
            };
            let mut buf = Vec::new();
            event.encode(&mut buf).unwrap();
            let decoded = KeyEvent::decode(&buf).unwrap();
            assert_eq!(decoded.keycode, expected_code);
            assert_eq!(char::from_u32(decoded.keycode), Some(ch));
        }

        // Special keysyms like XKB Backspace (0xff08)
        let backspace_xkb: u32 = 0xff08;
        let event = KeyEvent {
            action: KeyAction::Down,
            keycode: backspace_xkb,
            modifiers: 0,
            timestamp_us: 2000,
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        let decoded = KeyEvent::decode(&buf).unwrap();
        assert_eq!(decoded.keycode, 0xff08);
    }

    #[test]
    fn test_scroll_event_round_trip() {
        let event = ScrollEvent {
            dx: 0,
            dy: -120,
            x_norm: 32768,
            y_norm: 32768,
            timestamp_us: 555_666_777,
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), SCROLL_EVENT_SIZE);
        let decoded = ScrollEvent::decode(&buf).unwrap();
        assert_eq!(event, decoded);
    }

    #[test]
    fn test_nav_event_round_trip() {
        let event = NavEvent {
            action: NavAction::Back,
            timestamp_us: 111_222_333,
        };
        let mut buf = Vec::new();
        event.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), NAV_EVENT_SIZE);
        let decoded = NavEvent::decode(&buf).unwrap();
        assert_eq!(event, decoded);
    }

    #[test]
    fn test_input_ack_round_trip() {
        let ack = InputAck {
            seq: 42,
            timestamp_us: 888_999_000,
        };
        let mut buf = Vec::new();
        ack.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), INPUT_ACK_SIZE);
        let decoded = InputAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }
}
