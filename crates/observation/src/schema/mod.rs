//! Schema dispatch and version constant.
//!
//! Archival DTOs live in [`v1`] (module name retained). Version 3 is retained as a read-only
//! compatibility shape via serde defaults; version 4 is emitted for Phase I hazard vocabulary.

pub const MIN_SUPPORTED_SCHEMA_VERSION: u32 = 3;
pub const CURRENT_SCHEMA_VERSION: u32 = 4;

pub fn is_supported_schema_version(version: u32) -> bool {
    (MIN_SUPPORTED_SCHEMA_VERSION..=CURRENT_SCHEMA_VERSION).contains(&version)
}

pub mod v1;
