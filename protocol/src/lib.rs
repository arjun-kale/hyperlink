//! HyperLink Protocol — shared wire types, versioning, and measurement structures.
//!
//! This crate is the source of truth for the protocol contract between
//! `hyperlink-linux` (Rust host) and `hyperlink-android` (Kotlin companion).
//!
//! Phase 0 provides: protocol header, message types for clock sync and echo,
//! and structured measurement/reporting types used by `hyperlink-bench`.
//!
//! FlatBuffers (hot-path) and Protobuf (control-plane) schemas arrive in Phase 1.

pub mod clock;
pub mod echo;
pub mod message;
pub mod metrics;
pub mod version;
