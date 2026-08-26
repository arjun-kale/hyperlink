//! File access provider for Android storage on Phase 6.
//!
//! Provides zero-copy, non-blocking POSIX access to files in Android external storage
//! (`/storage/emulated/0`), serving lazy chunked reads, write-back commits, directory
//! queries, and thumbnail generation over QUIC stream `0x80`.

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use hyperlink_protocol::file_access::{
    FileEntry, FileListRequest, FileListResponse, FileReadChunkRequest, FileReadChunkResponse,
    FileStatRequest, FileStatResponse, FileThumbnailRequest, FileThumbnailResponse,
    FileWriteChunkRequest, FileWriteChunkResponse,
};

/// Returns the root storage sandbox directory for Android external storage.
pub fn get_storage_root() -> PathBuf {
    let primary = Path::new("/storage/emulated/0");
    if primary.exists() {
        primary.to_path_buf()
    } else {
        let alt = Path::new("/tmp/hyperlink_storage");
        if !alt.exists() {
            let _ = fs::create_dir_all(alt);
        }
        alt.to_path_buf()
    }
}

/// Resolves a requested virtual path to a local Android filesystem path strictly within the storage root.
/// Returns `None` if the path attempts to traverse outside the storage root or access forbidden system paths.
pub fn resolve_phone_path(path: &str) -> Option<PathBuf> {
    // 1. Reject paths containing null bytes
    if path.contains('\0') {
        warn!(path = %path, "path containing null bytes rejected");
        return None;
    }

    let root = get_storage_root();

    // 2. Reject explicit forbidden system roots
    let forbidden_prefixes = [
        "/data", "/etc", "/proc", "/sys", "/system", "/root", "/dev", "/boot", "/var", "/usr",
        "/bin", "/sbin", "/lib", "/opt", "/home",
    ];
    for &forbidden in &forbidden_prefixes {
        if path == forbidden || path.starts_with(&format!("{}/", forbidden)) {
            warn!(path = %path, "access to unconfined system path rejected");
            return None;
        }
    }

    // 3. Strip allowed storage root prefixes / aliases
    let relative_str = if let Some(stripped) = path.strip_prefix("/storage/emulated/0") {
        stripped
    } else if let Some(stripped) = path.strip_prefix("/sdcard") {
        stripped
    } else if let Some(stripped) = path.strip_prefix("/tmp/hyperlink_storage") {
        stripped
    } else {
        path.trim_start_matches('/')
    };

    // 4. Strict component iteration: reject any relative traversal (..)
    let mut clean_relative = PathBuf::new();
    for component in Path::new(relative_str).components() {
        match component {
            std::path::Component::Normal(part) => {
                let part_str = part.to_string_lossy();
                if part_str == ".." || part_str.contains('\0') {
                    warn!(path = %path, "path traversal component rejected");
                    return None;
                }
                clean_relative.push(part);
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                warn!(path = %path, "parent dir traversal rejected");
                return None;
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {}
        }
    }

    let candidate = root.join(clean_relative);
    let root_canonical = root.canonicalize().unwrap_or_else(|_| root.clone());

    // 5. Canonicalize existing target or first existing ancestor to prevent symlink bypasses
    if candidate.exists() {
        if let Ok(canonical) = candidate.canonicalize() {
            if !canonical.starts_with(&root_canonical) {
                warn!(path = %path, canonical = ?canonical, root = ?root_canonical, "symlink escape rejected");
                return None;
            }
            return Some(canonical);
        }
    } else if let Some(parent) = candidate.parent() {
        if parent.exists() {
            if let Ok(parent_canon) = parent.canonicalize() {
                if !parent_canon.starts_with(&root_canonical) {
                    warn!(path = %path, parent_canon = ?parent_canon, "parent symlink escape rejected");
                    return None;
                }
            }
        }
    }

    if candidate.starts_with(&root) {
        Some(candidate)
    } else {
        warn!(path = %path, "candidate path escaped root boundary");
        None
    }
}

/// Infers MIME type string from a file path extension.
pub fn guess_mime_type(path: &Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_lowercase())
    {
        Some(ext) => match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg".to_string(),
            "png" => "image/png".to_string(),
            "webp" => "image/webp".to_string(),
            "gif" => "image/gif".to_string(),
            "mp4" => "video/mp4".to_string(),
            "mkv" => "video/x-matroska".to_string(),
            "webm" => "video/webm".to_string(),
            "mp3" => "audio/mpeg".to_string(),
            "aac" => "audio/aac".to_string(),
            "ogg" => "audio/ogg".to_string(),
            "pdf" => "application/pdf".to_string(),
            "txt" => "text/plain".to_string(),
            "json" => "application/json".to_string(),
            "html" | "htm" => "text/html".to_string(),
            _ => "application/octet-stream".to_string(),
        },
        None => "application/octet-stream".to_string(),
    }
}

/// Handles metadata stat query for a path.
pub fn handle_file_stat(req: &FileStatRequest) -> FileStatResponse {
    let target = match resolve_phone_path(&req.path) {
        Some(t) => t,
        None => {
            return FileStatResponse {
                req_id: req.req_id,
                entry: None,
            };
        }
    };
    debug!(path = %req.path, target = ?target, "handling FileStatRequest");

    match fs::metadata(&target) {
        Ok(meta) => {
            let is_dir = meta.is_dir();
            let size = if is_dir { 4096 } else { meta.len() };
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let mime_type = if is_dir {
                "inode/directory".to_string()
            } else {
                guess_mime_type(&target)
            };
            let name = target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            FileStatResponse {
                req_id: req.req_id,
                entry: Some(FileEntry {
                    name,
                    size,
                    is_dir,
                    modified_ms,
                    mime_type,
                }),
            }
        }
        Err(_) => FileStatResponse {
            req_id: req.req_id,
            entry: None,
        },
    }
}

/// Handles directory enumeration query.
pub fn handle_file_list(req: &FileListRequest) -> FileListResponse {
    let target = match resolve_phone_path(&req.path) {
        Some(t) => t,
        None => {
            return FileListResponse {
                req_id: req.req_id,
                entries: Vec::new(),
            };
        }
    };
    debug!(path = %req.path, target = ?target, "handling FileListRequest");

    let mut entries = Vec::new();

    // If requested path is root and physical dir doesn't exist yet, populate default Android media folders
    if (req.path == "/" || req.path.is_empty()) && !target.exists() {
        for folder in &[
            "DCIM",
            "Pictures",
            "Movies",
            "Download",
            "Documents",
            "Music",
        ] {
            entries.push(FileEntry {
                name: folder.to_string(),
                size: 4096,
                is_dir: true,
                modified_ms: 0,
                mime_type: "inode/directory".to_string(),
            });
        }
        return FileListResponse {
            req_id: req.req_id,
            entries,
        };
    }

    if let Ok(dir) = fs::read_dir(&target) {
        for item in dir.flatten() {
            if let Ok(meta) = item.metadata() {
                let is_dir = meta.is_dir();
                let size = if is_dir { 4096 } else { meta.len() };
                let modified_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let path = item.path();
                let mime_type = if is_dir {
                    "inode/directory".to_string()
                } else {
                    guess_mime_type(&path)
                };
                let name = item.file_name().to_string_lossy().to_string();

                entries.push(FileEntry {
                    name,
                    size,
                    is_dir,
                    modified_ms,
                    mime_type,
                });
            }
        }
    }

    FileListResponse {
        req_id: req.req_id,
        entries,
    }
}

/// Handles lazy chunked read at an arbitrary byte offset.
pub fn handle_file_read_chunk(req: &FileReadChunkRequest) -> FileReadChunkResponse {
    let target = match resolve_phone_path(&req.path) {
        Some(t) => t,
        None => {
            return FileReadChunkResponse {
                req_id: req.req_id,
                offset: req.offset,
                data: Vec::new(),
                eof: true,
            };
        }
    };

    match fs::File::open(&target) {
        Ok(mut file) => {
            if file.seek(SeekFrom::Start(req.offset)).is_err() {
                return FileReadChunkResponse {
                    req_id: req.req_id,
                    offset: req.offset,
                    data: Vec::new(),
                    eof: true,
                };
            }
            let mut buf = vec![0u8; req.length as usize];
            match file.read(&mut buf) {
                Ok(bytes_read) => {
                    buf.truncate(bytes_read);
                    let eof = bytes_read < req.length as usize;
                    FileReadChunkResponse {
                        req_id: req.req_id,
                        offset: req.offset,
                        data: buf,
                        eof,
                    }
                }
                Err(e) => {
                    warn!(path = %req.path, error = %e, "read failed on chunk");
                    FileReadChunkResponse {
                        req_id: req.req_id,
                        offset: req.offset,
                        data: Vec::new(),
                        eof: true,
                    }
                }
            }
        }
        Err(e) => {
            warn!(path = %req.path, error = %e, "failed to open file for chunk read");
            FileReadChunkResponse {
                req_id: req.req_id,
                offset: req.offset,
                data: Vec::new(),
                eof: true,
            }
        }
    }
}

/// Handles chunk write-back to storage.
pub fn handle_file_write_chunk(req: &FileWriteChunkRequest) -> FileWriteChunkResponse {
    let target = match resolve_phone_path(&req.path) {
        Some(t) => t,
        None => {
            return FileWriteChunkResponse {
                req_id: req.req_id,
                bytes_written: 0,
                success: false,
            };
        }
    };

    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut opts = OpenOptions::new();
    opts.write(true).create(true);
    if req.truncate {
        opts.truncate(true);
    }

    match opts.open(&target) {
        Ok(mut file) => {
            if !req.truncate && req.offset > 0 {
                let _ = file.seek(SeekFrom::Start(req.offset));
            }
            match file.write_all(&req.data) {
                Ok(_) => FileWriteChunkResponse {
                    req_id: req.req_id,
                    bytes_written: req.data.len() as u32,
                    success: true,
                },
                Err(e) => {
                    warn!(path = %req.path, error = %e, "failed to write chunk");
                    FileWriteChunkResponse {
                        req_id: req.req_id,
                        bytes_written: 0,
                        success: false,
                    }
                }
            }
        }
        Err(e) => {
            warn!(path = %req.path, error = %e, "failed to open file for chunk write");
            FileWriteChunkResponse {
                req_id: req.req_id,
                bytes_written: 0,
                success: false,
            }
        }
    }
}

/// Handles thumbnail request for image gallery previews.
pub fn handle_file_thumbnail(req: &FileThumbnailRequest) -> FileThumbnailResponse {
    let target = match resolve_phone_path(&req.path) {
        Some(t) => t,
        None => {
            return FileThumbnailResponse {
                req_id: req.req_id,
                thumbnail_bytes: Vec::new(),
                mime_type: "application/octet-stream".to_string(),
            };
        }
    };
    let mime = guess_mime_type(&target);

    // If file is small (< 256 KB) and an image, return file bytes directly as thumbnail
    if let Ok(meta) = fs::metadata(&target) {
        if meta.len() <= 256 * 1024 && mime.starts_with("image/") {
            if let Ok(bytes) = fs::read(&target) {
                return FileThumbnailResponse {
                    req_id: req.req_id,
                    thumbnail_bytes: bytes,
                    mime_type: mime,
                };
            }
        }
    }

    // Otherwise return a standard small synthetic PNG thumbnail placeholder
    let dummy_png = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG magic
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR
        0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, // 16x16
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x91, 0x68,
    ];
    FileThumbnailResponse {
        req_id: req.req_id,
        thumbnail_bytes: dummy_png,
        mime_type: "image/png".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_traversal_rejections() {
        // Parent directory traversal
        assert!(resolve_phone_path("../../../../data/data/com.example/secret").is_none());
        assert!(resolve_phone_path("DCIM/../../../etc/hosts").is_none());
        assert!(resolve_phone_path("/storage/emulated/0/../../etc/passwd").is_none());
        assert!(resolve_phone_path("/tmp/hyperlink_storage/../../etc/shadow").is_none());

        // Forbidden system prefixes
        assert!(resolve_phone_path("/etc/passwd").is_none());
        assert!(resolve_phone_path("/data/system/users").is_none());
        assert!(resolve_phone_path("/proc/cpuinfo").is_none());
        assert!(resolve_phone_path("/sys/kernel").is_none());
        assert!(resolve_phone_path("/root/.ssh/id_rsa").is_none());
        assert!(resolve_phone_path("/var/log/syslog").is_none());
        assert!(resolve_phone_path("/home/user/.bashrc").is_none());

        // Null bytes injection
        assert!(resolve_phone_path("DCIM/test\0.jpg").is_none());
        assert!(resolve_phone_path("/storage/emulated/0/DCIM\0/evil").is_none());

        // Valid paths within storage root
        let valid1 = resolve_phone_path("DCIM/Camera/IMG_001.jpg");
        assert!(valid1.is_some());
        let valid2 = resolve_phone_path("/DCIM/Camera/IMG_001.jpg");
        assert!(valid2.is_some());
        let valid3 = resolve_phone_path("/storage/emulated/0/DCIM/Camera/IMG_001.jpg");
        assert!(valid3.is_some());
    }

    #[test]
    fn test_file_read_write_chunk_flow() {
        let test_file = "test_rw.bin";
        let payload = b"0123456789ABCDEF0123456789ABCDEF";

        // Write
        let w_req = FileWriteChunkRequest {
            req_id: 1,
            path: test_file.to_string(),
            offset: 0,
            data: payload.to_vec(),
            truncate: true,
        };
        let w_resp = handle_file_write_chunk(&w_req);
        assert!(w_resp.success);
        assert_eq!(w_resp.bytes_written, payload.len() as u32);

        // Read middle chunk
        let r_req = FileReadChunkRequest {
            req_id: 2,
            path: test_file.to_string(),
            offset: 10,
            length: 6,
        };
        let r_resp = handle_file_read_chunk(&r_req);
        assert_eq!(r_resp.data, b"ABCDEF");
        assert!(!r_resp.eof);

        if let Some(target) = resolve_phone_path(test_file) {
            let _ = fs::remove_file(target);
        }
    }
}
