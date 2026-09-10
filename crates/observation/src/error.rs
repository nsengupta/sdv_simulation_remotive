use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ObservationError {
    #[error("observation I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("observation JSON failed at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid observation timestamp {value:?}: {reason}")]
    InvalidTimestamp { value: String, reason: String },
    #[error("unsupported observation schema version {found}; supported version is {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error("record schema version {row} does not match manifest schema version {manifest}")]
    SchemaVersionMismatch { manifest: u32, row: u32 },
    #[error("{stream} line {line}: {message}")]
    InvalidRecord {
        stream: PathBuf,
        line: usize,
        message: String,
    },
    #[error("invalid observation manifest: {message}")]
    InvalidManifest { message: String },
    #[error("run directory already exists: {0}")]
    RunAlreadyExists(PathBuf),
    #[error("record run ID {found} does not match manifest run ID {expected}")]
    RunIdMismatch { expected: String, found: String },
    #[error("record vehicle {found:?} does not match manifest vehicle {expected:?}")]
    VehicleMismatch { expected: String, found: String },
    #[error(
        "record session_started_at {found} does not match manifest session_started_at {expected}"
    )]
    SessionMismatch { expected: String, found: String },
    #[error("UDS path must be under <cwd>/tmp: {path} ({reason})")]
    InvalidUdsPath { path: PathBuf, reason: String },
    #[error("zenoh: {message}")]
    Zenoh { message: String },
}
