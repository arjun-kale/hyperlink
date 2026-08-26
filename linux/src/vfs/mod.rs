//! HyperLink Virtual Filesystem (VFS) and FUSE mount for Phase 6.
//!
//! Provides a lazy virtual filesystem exposing phone storage (DCIM, Pictures, Movies, Documents)
//! as a mounted directory on Linux with chunked random-access reads, LRU caching, and write-backs.

pub mod cache;
pub mod client;
pub mod fuse_mount;

pub use cache::FileChunkCache;
pub use client::VfsClient;
pub use fuse_mount::{mount_fuse, HyperLinkFuse};
