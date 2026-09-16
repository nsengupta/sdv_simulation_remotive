//! Schema dispatch and version constant.
//!
//! Archival DTOs live in [`v1`] (module name retained). Versions 1–4 remain readable:
//! missing SCCM/BCM objects default, and historical boolean fields deserialize as Off/On.
//! Version 5 emits tri-state observed values. Version 6 adds latched `hazard_mode_on`.

pub const MIN_SUPPORTED_SCHEMA_VERSION: u32 = 1;
pub const CURRENT_SCHEMA_VERSION: u32 = 6;

pub fn is_supported_schema_version(version: u32) -> bool {
    (MIN_SUPPORTED_SCHEMA_VERSION..=CURRENT_SCHEMA_VERSION).contains(&version)
}

pub mod v1;
