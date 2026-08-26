//! Remote VFS QUIC stream client for Phase 6.
//!
//! Synchronously or asynchronously interacts with the Android companion over dedicated
//! QUIC stream `0x80` to perform metadata queries, directory listing, lazy chunk reads,
//! write-backs, and thumbnail retrieval.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use hyperlink_protocol::file_access::{
    FileEntry, FileListRequest, FileListResponse, FileReadChunkRequest, FileReadChunkResponse,
    FileStatRequest, FileStatResponse, FileThumbnailRequest, FileThumbnailResponse,
    FileWriteChunkRequest, FileWriteChunkResponse,
};
use hyperlink_protocol::message::MessageType;
use hyperlink_protocol::version::Header;

/// Manages communication over dedicated QUIC stream 0x80.
pub struct VfsClient {
    next_req_id: AtomicU64,
    stream: Arc<Mutex<Option<(quinn::SendStream, quinn::RecvStream)>>>,
}

impl VfsClient {
    pub fn new() -> Self {
        Self {
            next_req_id: AtomicU64::new(1),
            stream: Arc::new(Mutex::new(None)),
        }
    }

    /// Attaches the active QUIC stream (after stream-type handshake).
    pub async fn set_stream(&self, send: quinn::SendStream, recv: quinn::RecvStream) {
        let mut guard = self.stream.lock().await;
        *guard = Some((send, recv));
    }

    /// Queries file metadata for a path.
    pub async fn stat(&self, path: &str) -> io::Result<Option<FileEntry>> {
        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let req = FileStatRequest {
            req_id,
            path: path.to_string(),
        };

        let mut payload = Vec::new();
        req.encode(&mut payload)?;
        let header = Header::new(MessageType::FileStatRequest, payload.len() as u32);
        let mut packet = Vec::with_capacity(10 + payload.len());
        header.encode(&mut packet)?;
        packet.extend_from_slice(&payload);

        let resp_payload = self.round_trip(&packet).await?;
        let resp = FileStatResponse::decode(&resp_payload)?;
        Ok(resp.entry)
    }

    /// Lists directory entries.
    pub async fn list(&self, path: &str) -> io::Result<Vec<FileEntry>> {
        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let req = FileListRequest {
            req_id,
            path: path.to_string(),
        };

        let mut payload = Vec::new();
        req.encode(&mut payload)?;
        let header = Header::new(MessageType::FileListRequest, payload.len() as u32);
        let mut packet = Vec::with_capacity(10 + payload.len());
        header.encode(&mut packet)?;
        packet.extend_from_slice(&payload);

        let resp_payload = self.round_trip(&packet).await?;
        let resp = FileListResponse::decode(&resp_payload)?;
        Ok(resp.entries)
    }

    /// Reads a chunk of bytes lazily at specified offset.
    pub async fn read_chunk(&self, path: &str, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let req = FileReadChunkRequest {
            req_id,
            path: path.to_string(),
            offset,
            length,
        };

        let mut payload = Vec::new();
        req.encode(&mut payload)?;
        let header = Header::new(MessageType::FileReadChunkRequest, payload.len() as u32);
        let mut packet = Vec::with_capacity(10 + payload.len());
        header.encode(&mut packet)?;
        packet.extend_from_slice(&payload);

        let resp_payload = self.round_trip(&packet).await?;
        let resp = FileReadChunkResponse::decode(&resp_payload)?;
        Ok(resp.data)
    }

    /// Writes a chunk of bytes back to remote storage.
    pub async fn write_chunk(
        &self,
        path: &str,
        offset: u64,
        data: &[u8],
        truncate: bool,
    ) -> io::Result<u32> {
        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let req = FileWriteChunkRequest {
            req_id,
            path: path.to_string(),
            offset,
            data: data.to_vec(),
            truncate,
        };

        let mut payload = Vec::new();
        req.encode(&mut payload)?;
        let header = Header::new(MessageType::FileWriteChunkRequest, payload.len() as u32);
        let mut packet = Vec::with_capacity(10 + payload.len());
        header.encode(&mut packet)?;
        packet.extend_from_slice(&payload);

        let resp_payload = self.round_trip(&packet).await?;
        let resp = FileWriteChunkResponse::decode(&resp_payload)?;
        if resp.success {
            Ok(resp.bytes_written)
        } else {
            Err(io::Error::other("remote write chunk failed"))
        }
    }

    /// Fetches a downscaled thumbnail.
    pub async fn thumbnail(
        &self,
        path: &str,
        max_width: u32,
        max_height: u32,
    ) -> io::Result<Vec<u8>> {
        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let req = FileThumbnailRequest {
            req_id,
            path: path.to_string(),
            max_width,
            max_height,
        };

        let mut payload = Vec::new();
        req.encode(&mut payload)?;
        let header = Header::new(MessageType::FileThumbnailRequest, payload.len() as u32);
        let mut packet = Vec::with_capacity(10 + payload.len());
        header.encode(&mut packet)?;
        packet.extend_from_slice(&payload);

        let resp_payload = self.round_trip(&packet).await?;
        let resp = FileThumbnailResponse::decode(&resp_payload)?;
        Ok(resp.thumbnail_bytes)
    }

    /// Executes request-response round-trip over stream 0x80.
    async fn round_trip(&self, packet: &[u8]) -> io::Result<Vec<u8>> {
        let mut guard = self.stream.lock().await;
        let (send, recv) = guard.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "VFS QUIC stream not connected")
        })?;

        send.write_all(packet)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;

        let mut hdr_bytes = [0u8; hyperlink_protocol::version::HEADER_SIZE];
        recv.read_exact(&mut hdr_bytes)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::UnexpectedEof, e))?;

        let header = Header::decode(&hdr_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut resp_payload = vec![0u8; header.payload_len as usize];
        recv.read_exact(&mut resp_payload)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::UnexpectedEof, e))?;

        Ok(resp_payload)
    }
}

impl Default for VfsClient {
    fn default() -> Self {
        Self::new()
    }
}
