//! File access and virtual filesystem mount wire protocol for Phase 6.
//!
//! Defines messages for remote file attribute query (`FileStat`), directory
//! enumeration (`FileList`), lazy chunked random-access reads (`FileReadChunk`),
//! chunked write-back (`FileWriteChunk`), and thumbnail pre-fetching (`FileThumbnail`).
//!
//! All payloads are strictly bounded to prevent memory exhaustion.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Cursor, Read};

/// Maximum allowable path length in bytes.
pub const MAX_PATH_LEN: usize = 1024;
/// Maximum MIME type string length in bytes.
pub const MAX_MIME_LEN: usize = 128;
/// Maximum payload chunk size for a single read/write request (1 MB).
pub const MAX_CHUNK_SIZE: usize = 1024 * 1024;
/// Maximum number of directory entries returned in a single list response.
pub const MAX_ENTRIES_COUNT: usize = 10000;

/// Directory or file metadata entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub modified_ms: u64,
    pub mime_type: String,
}

impl FileEntry {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        write_string(dst, &self.name, MAX_PATH_LEN)?;
        dst.write_u64::<BigEndian>(self.size)?;
        dst.write_u8(if self.is_dir { 1 } else { 0 })?;
        dst.write_u64::<BigEndian>(self.modified_ms)?;
        write_string(dst, &self.mime_type, MAX_MIME_LEN)?;
        Ok(())
    }

    pub fn decode(cursor: &mut Cursor<&[u8]>) -> io::Result<Self> {
        let name = read_string(cursor, MAX_PATH_LEN)?;
        let size = cursor.read_u64::<BigEndian>()?;
        let is_dir = cursor.read_u8()? != 0;
        let modified_ms = cursor.read_u64::<BigEndian>()?;
        let mime_type = read_string(cursor, MAX_MIME_LEN)?;
        Ok(Self {
            name,
            size,
            is_dir,
            modified_ms,
            mime_type,
        })
    }
}

/// Request metadata for a path (`host → phone`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatRequest {
    pub req_id: u64,
    pub path: String,
}

impl FileStatRequest {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.req_id)?;
        write_string(dst, &self.path, MAX_PATH_LEN)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let req_id = cursor.read_u64::<BigEndian>()?;
        let path = read_string(&mut cursor, MAX_PATH_LEN)?;
        Ok(Self { req_id, path })
    }
}

/// Response containing path metadata or None if not found (`phone → host`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatResponse {
    pub req_id: u64,
    pub entry: Option<FileEntry>,
}

impl FileStatResponse {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.req_id)?;
        match &self.entry {
            Some(entry) => {
                dst.write_u8(1)?;
                entry.encode(dst)?;
            }
            None => {
                dst.write_u8(0)?;
            }
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let req_id = cursor.read_u64::<BigEndian>()?;
        let exists = cursor.read_u8()? != 0;
        let entry = if exists {
            Some(FileEntry::decode(&mut cursor)?)
        } else {
            None
        };
        Ok(Self { req_id, entry })
    }
}

/// Request directory listing (`host → phone`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileListRequest {
    pub req_id: u64,
    pub path: String,
}

impl FileListRequest {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.req_id)?;
        write_string(dst, &self.path, MAX_PATH_LEN)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let req_id = cursor.read_u64::<BigEndian>()?;
        let path = read_string(&mut cursor, MAX_PATH_LEN)?;
        Ok(Self { req_id, path })
    }
}

/// Response containing directory entries (`phone → host`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileListResponse {
    pub req_id: u64,
    pub entries: Vec<FileEntry>,
}

impl FileListResponse {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.req_id)?;
        dst.write_u32::<BigEndian>(self.entries.len() as u32)?;
        for entry in &self.entries {
            entry.encode(dst)?;
        }
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let req_id = cursor.read_u64::<BigEndian>()?;
        let count = cursor.read_u32::<BigEndian>()? as usize;
        if count > MAX_ENTRIES_COUNT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "too many directory entries",
            ));
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(FileEntry::decode(&mut cursor)?);
        }
        Ok(Self { req_id, entries })
    }
}

/// Lazy chunk read request (`host → phone`).
///
/// Requests `length` bytes at `offset` without downloading the full file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReadChunkRequest {
    pub req_id: u64,
    pub path: String,
    pub offset: u64,
    pub length: u32,
}

impl FileReadChunkRequest {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.req_id)?;
        write_string(dst, &self.path, MAX_PATH_LEN)?;
        dst.write_u64::<BigEndian>(self.offset)?;
        dst.write_u32::<BigEndian>(self.length)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let req_id = cursor.read_u64::<BigEndian>()?;
        let path = read_string(&mut cursor, MAX_PATH_LEN)?;
        let offset = cursor.read_u64::<BigEndian>()?;
        let length = cursor.read_u32::<BigEndian>()?;
        if length as usize > MAX_CHUNK_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "requested chunk length exceeds maximum",
            ));
        }
        Ok(Self {
            req_id,
            path,
            offset,
            length,
        })
    }
}

/// Chunk read response (`phone → host`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReadChunkResponse {
    pub req_id: u64,
    pub offset: u64,
    pub data: Vec<u8>,
    pub eof: bool,
}

impl FileReadChunkResponse {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.req_id)?;
        dst.write_u64::<BigEndian>(self.offset)?;
        dst.write_u8(if self.eof { 1 } else { 0 })?;
        dst.write_u32::<BigEndian>(self.data.len() as u32)?;
        dst.extend_from_slice(&self.data);
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let req_id = cursor.read_u64::<BigEndian>()?;
        let offset = cursor.read_u64::<BigEndian>()?;
        let eof = cursor.read_u8()? != 0;
        let len = cursor.read_u32::<BigEndian>()? as usize;
        if len > MAX_CHUNK_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "read chunk payload exceeds maximum",
            ));
        }
        let mut chunk = vec![0u8; len];
        cursor.read_exact(&mut chunk)?;
        Ok(Self {
            req_id,
            offset,
            data: chunk,
            eof,
        })
    }
}

/// Write-back chunk request (`host → phone`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWriteChunkRequest {
    pub req_id: u64,
    pub path: String,
    pub offset: u64,
    pub data: Vec<u8>,
    pub truncate: bool,
}

impl FileWriteChunkRequest {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.req_id)?;
        write_string(dst, &self.path, MAX_PATH_LEN)?;
        dst.write_u64::<BigEndian>(self.offset)?;
        dst.write_u8(if self.truncate { 1 } else { 0 })?;
        dst.write_u32::<BigEndian>(self.data.len() as u32)?;
        dst.extend_from_slice(&self.data);
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let req_id = cursor.read_u64::<BigEndian>()?;
        let path = read_string(&mut cursor, MAX_PATH_LEN)?;
        let offset = cursor.read_u64::<BigEndian>()?;
        let truncate = cursor.read_u8()? != 0;
        let len = cursor.read_u32::<BigEndian>()? as usize;
        if len > MAX_CHUNK_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "write chunk payload exceeds maximum",
            ));
        }
        let mut chunk = vec![0u8; len];
        cursor.read_exact(&mut chunk)?;
        Ok(Self {
            req_id,
            path,
            offset,
            data: chunk,
            truncate,
        })
    }
}

/// Write chunk response (`phone → host`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWriteChunkResponse {
    pub req_id: u64,
    pub bytes_written: u32,
    pub success: bool,
}

impl FileWriteChunkResponse {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.req_id)?;
        dst.write_u32::<BigEndian>(self.bytes_written)?;
        dst.write_u8(if self.success { 1 } else { 0 })?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let req_id = cursor.read_u64::<BigEndian>()?;
        let bytes_written = cursor.read_u32::<BigEndian>()?;
        let success = cursor.read_u8()? != 0;
        Ok(Self {
            req_id,
            bytes_written,
            success,
        })
    }
}

/// Thumbnail request for image/video gallery browsing (`host → phone`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileThumbnailRequest {
    pub req_id: u64,
    pub path: String,
    pub max_width: u32,
    pub max_height: u32,
}

impl FileThumbnailRequest {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.req_id)?;
        write_string(dst, &self.path, MAX_PATH_LEN)?;
        dst.write_u32::<BigEndian>(self.max_width)?;
        dst.write_u32::<BigEndian>(self.max_height)?;
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let req_id = cursor.read_u64::<BigEndian>()?;
        let path = read_string(&mut cursor, MAX_PATH_LEN)?;
        let max_width = cursor.read_u32::<BigEndian>()?;
        let max_height = cursor.read_u32::<BigEndian>()?;
        Ok(Self {
            req_id,
            path,
            max_width,
            max_height,
        })
    }
}

/// Thumbnail response carrying scaled down image payload (`phone → host`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileThumbnailResponse {
    pub req_id: u64,
    pub thumbnail_bytes: Vec<u8>,
    pub mime_type: String,
}

impl FileThumbnailResponse {
    pub fn encode(&self, dst: &mut Vec<u8>) -> io::Result<()> {
        dst.write_u64::<BigEndian>(self.req_id)?;
        write_string(dst, &self.mime_type, MAX_MIME_LEN)?;
        dst.write_u32::<BigEndian>(self.thumbnail_bytes.len() as u32)?;
        dst.extend_from_slice(&self.thumbnail_bytes);
        Ok(())
    }

    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let req_id = cursor.read_u64::<BigEndian>()?;
        let mime_type = read_string(&mut cursor, MAX_MIME_LEN)?;
        let len = cursor.read_u32::<BigEndian>()? as usize;
        if len > MAX_CHUNK_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "thumbnail payload exceeds maximum",
            ));
        }
        let mut thumbnail_bytes = vec![0u8; len];
        cursor.read_exact(&mut thumbnail_bytes)?;
        Ok(Self {
            req_id,
            thumbnail_bytes,
            mime_type,
        })
    }
}

fn write_string(dst: &mut Vec<u8>, s: &str, max_len: usize) -> io::Result<()> {
    let bytes = s.as_bytes();
    if bytes.len() > max_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("string exceeds max length of {max_len}"),
        ));
    }
    dst.write_u16::<BigEndian>(bytes.len() as u16)?;
    dst.extend_from_slice(bytes);
    Ok(())
}

fn read_string(cursor: &mut Cursor<&[u8]>, max_len: usize) -> io::Result<String> {
    let len = cursor.read_u16::<BigEndian>()? as usize;
    if len > max_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("string exceeds max length of {max_len}"),
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
    fn test_file_stat_round_trip() {
        let req = FileStatRequest {
            req_id: 42,
            path: "/storage/emulated/0/DCIM/Camera/IMG_001.jpg".to_string(),
        };
        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();
        let decoded = FileStatRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);

        let resp = FileStatResponse {
            req_id: 42,
            entry: Some(FileEntry {
                name: "IMG_001.jpg".to_string(),
                size: 3_456_789,
                is_dir: false,
                modified_ms: 1_700_000_000_000,
                mime_type: "image/jpeg".to_string(),
            }),
        };
        let mut resp_buf = Vec::new();
        resp.encode(&mut resp_buf).unwrap();
        let decoded_resp = FileStatResponse::decode(&resp_buf).unwrap();
        assert_eq!(resp, decoded_resp);
    }

    #[test]
    fn test_file_list_round_trip() {
        let req = FileListRequest {
            req_id: 101,
            path: "/storage/emulated/0/DCIM".to_string(),
        };
        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();
        let decoded = FileListRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);

        let resp = FileListResponse {
            req_id: 101,
            entries: vec![
                FileEntry {
                    name: "Camera".to_string(),
                    size: 4096,
                    is_dir: true,
                    modified_ms: 1_700_000_000_000,
                    mime_type: "inode/directory".to_string(),
                },
                FileEntry {
                    name: "test.mp4".to_string(),
                    size: 52_428_800,
                    is_dir: false,
                    modified_ms: 1_700_000_100_000,
                    mime_type: "video/mp4".to_string(),
                },
            ],
        };
        let mut resp_buf = Vec::new();
        resp.encode(&mut resp_buf).unwrap();
        let decoded_resp = FileListResponse::decode(&resp_buf).unwrap();
        assert_eq!(resp, decoded_resp);
    }

    #[test]
    fn test_file_chunk_read_write_round_trip() {
        let read_req = FileReadChunkRequest {
            req_id: 201,
            path: "/storage/emulated/0/Movies/large.mp4".to_string(),
            offset: 1_000_000,
            length: 65536,
        };
        let mut r_buf = Vec::new();
        read_req.encode(&mut r_buf).unwrap();
        let decoded_r = FileReadChunkRequest::decode(&r_buf).unwrap();
        assert_eq!(read_req, decoded_r);

        let read_resp = FileReadChunkResponse {
            req_id: 201,
            offset: 1_000_000,
            data: vec![0x42; 65536],
            eof: false,
        };
        let mut r_resp_buf = Vec::new();
        read_resp.encode(&mut r_resp_buf).unwrap();
        let decoded_r_resp = FileReadChunkResponse::decode(&r_resp_buf).unwrap();
        assert_eq!(read_resp, decoded_r_resp);

        let write_req = FileWriteChunkRequest {
            req_id: 301,
            path: "/storage/emulated/0/Documents/notes.txt".to_string(),
            offset: 0,
            data: b"Hello HyperLink VFS".to_vec(),
            truncate: true,
        };
        let mut w_buf = Vec::new();
        write_req.encode(&mut w_buf).unwrap();
        let decoded_w = FileWriteChunkRequest::decode(&w_buf).unwrap();
        assert_eq!(write_req, decoded_w);

        let write_resp = FileWriteChunkResponse {
            req_id: 301,
            bytes_written: 19,
            success: true,
        };
        let mut w_resp_buf = Vec::new();
        write_resp.encode(&mut w_resp_buf).unwrap();
        let decoded_w_resp = FileWriteChunkResponse::decode(&w_resp_buf).unwrap();
        assert_eq!(write_resp, decoded_w_resp);
    }

    #[test]
    fn test_file_thumbnail_round_trip() {
        let req = FileThumbnailRequest {
            req_id: 401,
            path: "/storage/emulated/0/DCIM/Camera/IMG_999.jpg".to_string(),
            max_width: 256,
            max_height: 256,
        };
        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();
        let decoded = FileThumbnailRequest::decode(&buf).unwrap();
        assert_eq!(req, decoded);

        let resp = FileThumbnailResponse {
            req_id: 401,
            thumbnail_bytes: vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10],
            mime_type: "image/jpeg".to_string(),
        };
        let mut resp_buf = Vec::new();
        resp.encode(&mut resp_buf).unwrap();
        let decoded_resp = FileThumbnailResponse::decode(&resp_buf).unwrap();
        assert_eq!(resp, decoded_resp);
    }
}
