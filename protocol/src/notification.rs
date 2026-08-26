//! Notification sync wire protocol types for Phase 4.
//!
//! Encapsulates posted notifications from the Android phone, dismissal events,
//! action invocations (e.g. quick reply / action clicks), Do-Not-Disturb state,
//! and acknowledgments for latency accounting.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};
use std::io::{self, Cursor, Read};

/// Maximum allowed length for strings in notification payloads to prevent DoS.
const MAX_STRING_LEN: usize = 4096;
/// Maximum allowed icon PNG payload size (32 KB).
const MAX_ICON_SIZE: usize = 32 * 1024;
/// Maximum number of notification actions.
const MAX_ACTIONS: usize = 8;

/// An interactive action attached to a notification (e.g. "Reply", "Archive").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationAction {
    pub action_id: u32,
    pub title: String,
}

impl NotificationAction {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u32::<BigEndian>(self.action_id)?;
        let title_bytes = self.title.as_bytes();
        dst.write_u16::<BigEndian>(title_bytes.len() as u16)?;
        dst.extend_from_slice(title_bytes);
        Ok(())
    }

    pub fn decode(cursor: &mut Cursor<&[u8]>) -> io::Result<Self> {
        let action_id = cursor.read_u32::<BigEndian>()?;
        let title_len = cursor.read_u16::<BigEndian>()? as usize;
        if title_len > MAX_STRING_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "action title too long",
            ));
        }
        let mut title_bytes = vec![0u8; title_len];
        cursor.read_exact(&mut title_bytes)?;
        let title = String::from_utf8(title_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Self { action_id, title })
    }
}

/// Notification posted on the Android phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPost {
    pub id: String,
    pub package_name: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub timestamp_ms: u64,
    pub actions: Vec<NotificationAction>,
    pub icon_png: Option<Vec<u8>>,
}

impl NotificationPost {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        write_string(dst, &self.id)?;
        write_string(dst, &self.package_name)?;
        write_string(dst, &self.app_name)?;
        write_string(dst, &self.title)?;
        write_string(dst, &self.body)?;
        dst.write_u64::<BigEndian>(self.timestamp_ms)?;

        // Actions
        let action_count = self.actions.len().min(MAX_ACTIONS);
        dst.write_u8(action_count as u8)?;
        for action in self.actions.iter().take(action_count) {
            action.encode(dst)?;
        }

        // Icon
        match &self.icon_png {
            Some(icon) if icon.len() <= MAX_ICON_SIZE => {
                dst.write_u32::<BigEndian>(icon.len() as u32)?;
                dst.extend_from_slice(icon);
            }
            _ => {
                dst.write_u32::<BigEndian>(0)?;
            }
        }

        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let id = read_string(&mut cursor)?;
        let package_name = read_string(&mut cursor)?;
        let app_name = read_string(&mut cursor)?;
        let title = read_string(&mut cursor)?;
        let body = read_string(&mut cursor)?;
        let timestamp_ms = cursor.read_u64::<BigEndian>()?;

        let action_count = cursor.read_u8()? as usize;
        if action_count > MAX_ACTIONS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "too many actions",
            ));
        }
        let mut actions = Vec::with_capacity(action_count);
        for _ in 0..action_count {
            actions.push(NotificationAction::decode(&mut cursor)?);
        }

        let icon_len = cursor.read_u32::<BigEndian>()? as usize;
        let icon_png = if icon_len > 0 {
            if icon_len > MAX_ICON_SIZE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "icon size exceeds maximum",
                ));
            }
            let mut icon_bytes = vec![0u8; icon_len];
            cursor.read_exact(&mut icon_bytes)?;
            Some(icon_bytes)
        } else {
            None
        };

        Ok(Self {
            id,
            package_name,
            app_name,
            title,
            body,
            timestamp_ms,
            actions,
            icon_png,
        })
    }
}

/// Notification dismissed on the device or host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationDismiss {
    pub id: String,
}

impl NotificationDismiss {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        write_string(dst, &self.id)
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let id = read_string(&mut cursor)?;
        Ok(Self { id })
    }
}

/// Action invocation sent from Linux host to Android.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationActionInvoke {
    pub id: String,
    pub action_id: u32,
}

impl NotificationActionInvoke {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        write_string(dst, &self.id)?;
        dst.write_u32::<BigEndian>(self.action_id)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let id = read_string(&mut cursor)?;
        let action_id = cursor.read_u32::<BigEndian>()?;
        Ok(Self { id, action_id })
    }
}

/// Do-Not-Disturb state synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DndSync {
    pub dnd_enabled: bool,
}

impl DndSync {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u8(if self.dnd_enabled { 1 } else { 0 })
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        if data.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "dnd sync empty",
            ));
        }
        Ok(Self {
            dnd_enabled: data[0] != 0,
        })
    }
}

/// Acknowledgment sent by Linux host upon receiving a notification, measuring transit latency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationAck {
    pub id: String,
    pub device_timestamp_ms: u64,
    pub host_received_us: u64,
}

impl NotificationAck {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        write_string(dst, &self.id)?;
        dst.write_u64::<BigEndian>(self.device_timestamp_ms)?;
        dst.write_u64::<BigEndian>(self.host_received_us)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let id = read_string(&mut cursor)?;
        let device_timestamp_ms = cursor.read_u64::<BigEndian>()?;
        let host_received_us = cursor.read_u64::<BigEndian>()?;
        Ok(Self {
            id,
            device_timestamp_ms,
            host_received_us,
        })
    }
}

// Helper functions for string encoding
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
    fn test_notification_post_round_trip() {
        let post = NotificationPost {
            id: "sbn_key_12345".to_string(),
            package_name: "com.whatsapp".to_string(),
            app_name: "WhatsApp".to_string(),
            title: "Alice".to_string(),
            body: "Hey, are you free for a call?".to_string(),
            timestamp_ms: 1724688000000,
            actions: vec![
                NotificationAction {
                    action_id: 1,
                    title: "Reply".to_string(),
                },
                NotificationAction {
                    action_id: 2,
                    title: "Mark as Read".to_string(),
                },
            ],
            icon_png: Some(vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
        };

        let mut buf = Vec::new();
        post.encode(&mut buf).unwrap();
        let decoded = NotificationPost::decode(&buf).unwrap();
        assert_eq!(post, decoded);
    }

    #[test]
    fn test_notification_dismiss_round_trip() {
        let dismiss = NotificationDismiss {
            id: "sbn_key_12345".to_string(),
        };
        let mut buf = Vec::new();
        dismiss.encode(&mut buf).unwrap();
        let decoded = NotificationDismiss::decode(&buf).unwrap();
        assert_eq!(dismiss, decoded);
    }

    #[test]
    fn test_action_invoke_round_trip() {
        let invoke = NotificationActionInvoke {
            id: "sbn_key_12345".to_string(),
            action_id: 1,
        };
        let mut buf = Vec::new();
        invoke.encode(&mut buf).unwrap();
        let decoded = NotificationActionInvoke::decode(&buf).unwrap();
        assert_eq!(invoke, decoded);
    }

    #[test]
    fn test_dnd_sync_round_trip() {
        let dnd = DndSync { dnd_enabled: true };
        let mut buf = Vec::new();
        dnd.encode(&mut buf).unwrap();
        let decoded = DndSync::decode(&buf).unwrap();
        assert_eq!(dnd, decoded);

        let dnd_off = DndSync { dnd_enabled: false };
        let mut buf2 = Vec::new();
        dnd_off.encode(&mut buf2).unwrap();
        let decoded2 = DndSync::decode(&buf2).unwrap();
        assert_eq!(dnd_off, decoded2);
    }

    #[test]
    fn test_notification_ack_round_trip() {
        let ack = NotificationAck {
            id: "sbn_key_12345".to_string(),
            device_timestamp_ms: 1724688000000,
            host_received_us: 1724688000050000,
        };
        let mut buf = Vec::new();
        ack.encode(&mut buf).unwrap();
        let decoded = NotificationAck::decode(&buf).unwrap();
        assert_eq!(ack, decoded);
    }
}
