//! FUSE filesystem implementation for Phase 6 (Lazy Virtual Mount).
//!
//! Exposes the remote phone storage as a standard mounted directory on Linux,
//! performing lazy chunked reads, LRU caching, and write-back synchronization.

use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, Generation, INodeNo, LockOwner,
    OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyWrite, Request, WriteFlags,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use tracing::{error, info};

use super::cache::{FileChunkCache, CHUNK_SIZE};
use super::client::VfsClient;

const TTL: Duration = Duration::from_secs(1);

/// Inode metadata record.
#[derive(Debug, Clone)]
struct InodeData {
    path: String,
    is_dir: bool,
    size: u64,
    modified_ms: u64,
}

/// HyperLink Virtual Filesystem backed by FUSE.
pub struct HyperLinkFuse {
    client: Arc<VfsClient>,
    cache: Arc<FileChunkCache>,
    next_ino: AtomicU64,
    inodes: Mutex<HashMap<u64, InodeData>>,
    paths: Mutex<HashMap<String, u64>>,
    runtime: tokio::runtime::Handle,
}

impl HyperLinkFuse {
    pub fn new(
        client: Arc<VfsClient>,
        cache: Arc<FileChunkCache>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let mut inodes = HashMap::new();
        let mut paths = HashMap::new();

        // Inode 1 is the virtual filesystem root
        inodes.insert(
            1,
            InodeData {
                path: "/".to_string(),
                is_dir: true,
                size: 4096,
                modified_ms: 0,
            },
        );
        paths.insert("/".to_string(), 1);

        Self {
            client,
            cache,
            next_ino: AtomicU64::new(2),
            inodes: Mutex::new(inodes),
            paths: Mutex::new(paths),
            runtime,
        }
    }

    fn get_or_create_ino(&self, path: &str, is_dir: bool, size: u64, modified_ms: u64) -> u64 {
        let mut paths = self.paths.lock().unwrap();
        if let Some(&ino) = paths.get(path) {
            let mut inodes = self.inodes.lock().unwrap();
            if let Some(data) = inodes.get_mut(&ino) {
                data.size = size;
                data.modified_ms = modified_ms;
            }
            return ino;
        }

        let ino = self.next_ino.fetch_add(1, Ordering::Relaxed);
        paths.insert(path.to_string(), ino);

        let mut inodes = self.inodes.lock().unwrap();
        inodes.insert(
            ino,
            InodeData {
                path: path.to_string(),
                is_dir,
                size,
                modified_ms,
            },
        );
        ino
    }

    fn make_attr(&self, ino: u64, data: &InodeData) -> FileAttr {
        let mtime = UNIX_EPOCH + Duration::from_millis(data.modified_ms);
        let kind = if data.is_dir {
            FileType::Directory
        } else {
            FileType::RegularFile
        };
        let perm = if data.is_dir { 0o755 } else { 0o644 };

        FileAttr {
            ino: INodeNo(ino),
            size: data.size,
            blocks: data.size.div_ceil(512),
            atime: mtime,
            mtime,
            ctime: mtime,
            crtime: mtime,
            kind,
            perm,
            nlink: if data.is_dir { 2 } else { 1 },
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            flags: 0,
            blksize: CHUNK_SIZE as u32,
        }
    }
}

impl Filesystem for HyperLinkFuse {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(Errno::ENOENT);
                return;
            }
        };

        let parent_path = {
            let inodes = self.inodes.lock().unwrap();
            match inodes.get(&parent.0) {
                Some(data) => data.path.clone(),
                None => {
                    reply.error(Errno::ENOENT);
                    return;
                }
            }
        };

        let child_path = if parent_path == "/" {
            format!("/{name_str}")
        } else {
            format!("{parent_path}/{name_str}")
        };

        let client = self.client.clone();
        let query_path = child_path.clone();
        let stat_res = self
            .runtime
            .block_on(async move { client.stat(&query_path).await });

        match stat_res {
            Ok(Some(entry)) => {
                let ino = self.get_or_create_ino(
                    &child_path,
                    entry.is_dir,
                    entry.size,
                    entry.modified_ms,
                );
                let inodes = self.inodes.lock().unwrap();
                let data = inodes.get(&ino).unwrap();
                reply.entry(&TTL, &self.make_attr(ino, data), Generation(0));
            }
            _ => reply.error(Errno::ENOENT),
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        let (file_path, is_root) = {
            let inodes = self.inodes.lock().unwrap();
            match inodes.get(&ino.0) {
                Some(data) => (data.path.clone(), ino.0 == 1),
                None => {
                    reply.error(Errno::ENOENT);
                    return;
                }
            }
        };

        if is_root {
            let inodes = self.inodes.lock().unwrap();
            let data = inodes.get(&ino.0).unwrap();
            reply.attr(&TTL, &self.make_attr(ino.0, data));
            return;
        }

        // Query remote phone to refresh metadata and detect remote modifications
        let client = self.client.clone();
        let query_path = file_path.clone();
        let stat_res = self
            .runtime
            .block_on(async move { client.stat(&query_path).await });

        match stat_res {
            Ok(Some(entry)) => {
                let mut inodes = self.inodes.lock().unwrap();
                if let Some(data) = inodes.get_mut(&ino.0) {
                    if data.modified_ms != entry.modified_ms {
                        // Remote file was modified: invalidate cached chunks for fresh read
                        self.cache.invalidate_path(&file_path);
                    }
                    data.size = entry.size;
                    data.modified_ms = entry.modified_ms;
                    data.is_dir = entry.is_dir;
                    reply.attr(&TTL, &self.make_attr(ino.0, data));
                } else {
                    reply.error(Errno::ENOENT);
                }
            }
            Ok(None) => reply.error(Errno::ENOENT),
            Err(_) => {
                // Fallback to local cached metadata if network read fails
                let inodes = self.inodes.lock().unwrap();
                if let Some(data) = inodes.get(&ino.0) {
                    reply.attr(&TTL, &self.make_attr(ino.0, data));
                } else {
                    reply.error(Errno::EIO);
                }
            }
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<std::time::SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<std::time::SystemTime>,
        _chgtime: Option<std::time::SystemTime>,
        _bkuptime: Option<std::time::SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let (file_path, current_size) = {
            let inodes = self.inodes.lock().unwrap();
            match inodes.get(&ino.0) {
                Some(data) => (data.path.clone(), data.size),
                None => {
                    reply.error(Errno::ENOENT);
                    return;
                }
            }
        };

        if let Some(new_size) = size {
            if new_size == 0 {
                // Truncate file remotely
                let client = self.client.clone();
                let path = file_path.clone();
                let trunc_res = self
                    .runtime
                    .block_on(async move { client.write_chunk(&path, 0, &[], true).await });

                match trunc_res {
                    Ok(_) => {
                        self.cache.invalidate_path(&file_path);
                        let mut inodes = self.inodes.lock().unwrap();
                        if let Some(data) = inodes.get_mut(&ino.0) {
                            data.size = 0;
                            data.modified_ms = std::time::SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;
                            reply.attr(&TTL, &self.make_attr(ino.0, data));
                            return;
                        }
                    }
                    _ => {
                        reply.error(Errno::EIO);
                        return;
                    }
                }
            } else if new_size < current_size {
                // Truncation/shrink: invalidate cache and update size
                self.cache.invalidate_path(&file_path);
                let mut inodes = self.inodes.lock().unwrap();
                if let Some(data) = inodes.get_mut(&ino.0) {
                    data.size = new_size;
                    reply.attr(&TTL, &self.make_attr(ino.0, data));
                    return;
                }
            }
        }

        let inodes = self.inodes.lock().unwrap();
        if let Some(data) = inodes.get(&ino.0) {
            reply.attr(&TTL, &self.make_attr(ino.0, data));
        } else {
            reply.error(Errno::ENOENT);
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let dir_path = {
            let inodes = self.inodes.lock().unwrap();
            match inodes.get(&ino.0) {
                Some(data) if data.is_dir => data.path.clone(),
                _ => {
                    reply.error(Errno::ENOTDIR);
                    return;
                }
            }
        };

        let client = self.client.clone();
        let query_path = dir_path.clone();
        let list_res = self
            .runtime
            .block_on(async move { client.list(&query_path).await });

        let entries = match list_res {
            Ok(e) => e,
            Err(_) => {
                reply.error(Errno::EIO);
                return;
            }
        };

        // Standard "." and ".." entries
        let mut idx: u64 = 1;
        if offset < idx && reply.add(ino, idx, FileType::Directory, ".") {
            reply.ok();
            return;
        }
        idx += 1;

        if offset < idx && reply.add(INodeNo(1), idx, FileType::Directory, "..") {
            reply.ok();
            return;
        }
        idx += 1;

        for entry in entries {
            if offset < idx {
                let child_path = if dir_path == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", dir_path, entry.name)
                };

                let child_ino = self.get_or_create_ino(
                    &child_path,
                    entry.is_dir,
                    entry.size,
                    entry.modified_ms,
                );
                let kind = if entry.is_dir {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                };

                if reply.add(INodeNo(child_ino), idx, kind, &entry.name) {
                    break;
                }
            }
            idx += 1;
        }

        reply.ok();
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let (file_path, file_size) = {
            let inodes = self.inodes.lock().unwrap();
            match inodes.get(&ino.0) {
                Some(data) if !data.is_dir => (data.path.clone(), data.size),
                _ => {
                    reply.error(Errno::EISDIR);
                    return;
                }
            }
        };

        if offset >= file_size {
            reply.data(&[]);
            return;
        }

        let max_readable = (file_size - offset).min(size as u64) as usize;
        let mut result_buf = Vec::with_capacity(max_readable);

        let mut current_offset = offset;
        while result_buf.len() < max_readable {
            let chunk_idx = current_offset / CHUNK_SIZE as u64;
            let chunk_offset = chunk_idx * CHUNK_SIZE as u64;
            let offset_in_chunk = (current_offset - chunk_offset) as usize;

            // 1. Check local LRU chunk cache
            let chunk_data = match self.cache.get(&file_path, chunk_idx) {
                Some(cached) => cached,
                None => {
                    // 2. Fetch missing chunk from phone over QUIC stream 0x80
                    let client = self.client.clone();
                    let path = file_path.clone();
                    let fetch_len = CHUNK_SIZE as u32;

                    let fetch_res = self.runtime.block_on(async move {
                        client.read_chunk(&path, chunk_offset, fetch_len).await
                    });

                    match fetch_res {
                        Ok(data) => {
                            self.cache.put(&file_path, chunk_idx, data.clone());
                            data
                        }
                        Err(e) => {
                            error!(path = %file_path, offset = chunk_offset, error = %e, "failed to read chunk from phone");
                            reply.error(Errno::EIO);
                            return;
                        }
                    }
                }
            };

            if offset_in_chunk >= chunk_data.len() {
                break;
            }

            let available_in_chunk = chunk_data.len() - offset_in_chunk;
            let to_copy = available_in_chunk.min(max_readable - result_buf.len());
            result_buf.extend_from_slice(&chunk_data[offset_in_chunk..offset_in_chunk + to_copy]);
            current_offset += to_copy as u64;
        }

        reply.data(&result_buf);
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let file_path = {
            let inodes = self.inodes.lock().unwrap();
            match inodes.get(&ino.0) {
                Some(data) if !data.is_dir => data.path.clone(),
                _ => {
                    reply.error(Errno::EISDIR);
                    return;
                }
            }
        };

        let client = self.client.clone();
        let path = file_path.clone();
        let payload = data.to_vec();

        // Write-back: sync write chunk to phone
        let write_res = self
            .runtime
            .block_on(async move { client.write_chunk(&path, offset, &payload, false).await });

        match write_res {
            Ok(bytes_written) => {
                // Invalidate local chunk cache for this path to guarantee fresh reads
                self.cache.invalidate_path(&file_path);

                // Update size if write extends beyond current size
                let new_size = offset + bytes_written as u64;
                let mut inodes = self.inodes.lock().unwrap();
                if let Some(d) = inodes.get_mut(&ino.0) {
                    if new_size > d.size {
                        d.size = new_size;
                    }
                }

                reply.written(bytes_written);
            }
            Err(e) => {
                error!(path = %file_path, offset = offset, error = %e, "write-back failed");
                reply.error(Errno::EIO);
            }
        }
    }
}

/// Spawns the FUSE filesystem background mount session.
pub fn mount_fuse(
    mountpoint: &str,
    client: Arc<VfsClient>,
    cache: Arc<FileChunkCache>,
    runtime: tokio::runtime::Handle,
) -> anyhow::Result<fuser::BackgroundSession> {
    info!(mountpoint = %mountpoint, "mounting HyperLink virtual filesystem");
    std::fs::create_dir_all(mountpoint)?;

    let fs = HyperLinkFuse::new(client, cache, runtime);
    let mut config = Config::default();
    config.mount_options = vec![
        fuser::MountOption::FSName("hyperlink".to_string()),
        fuser::MountOption::AutoUnmount,
    ];

    let session = fuser::spawn_mount(fs, mountpoint, &config)?;
    info!(mountpoint = %mountpoint, "HyperLink virtual filesystem mounted successfully");
    Ok(session)
}
