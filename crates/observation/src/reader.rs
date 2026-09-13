//! Manifest-validated, lazy readers for a captured run directory.
//!
//! `RunReader::open` never trusts the manifest's declared schema version until it has parsed the
//! manifest as an untyped `serde_json::Value`, extracted `schema_version`, and confirmed it is a
//! deliberately supported version (v1–v5) — see `docs/DESIGN.md`. Only then
//! does it deserialize the concrete `ManifestV1` and validate the two declared stream filenames.
//! Every stream row is re-validated the same way as it is read, one line at a time, so corrupt or
//! foreign artifacts fail with contextual errors instead of silently loading.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Lines};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::ObservationError;
use crate::schema::v1::{
    DiagnosticPayloadV1, LedgerPayloadV1, ManifestV1, RunId, StreamEnvelopeV1, StreamsV1,
    UnixTimestampV1,
};
use crate::schema::{CURRENT_SCHEMA_VERSION, is_supported_schema_version};

const MANIFEST_FILE_NAME: &str = "manifest.json";
const DIAGNOSTIC_FILE_NAME: &str = "diagnostic.jsonl";
const LEDGER_FILE_NAME: &str = "ledger.jsonl";

/// A validated run directory: manifest already schema-checked and stream filenames confirmed.
#[derive(Debug)]
pub struct RunReader {
    run_dir: PathBuf,
    manifest: ManifestV1,
}

/// A fully loaded run: the manifest plus every diagnostic and ledger row, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRun {
    pub manifest: ManifestV1,
    pub diagnostics: Vec<StreamEnvelopeV1<DiagnosticPayloadV1>>,
    pub ledger: Vec<StreamEnvelopeV1<LedgerPayloadV1>>,
}

/// A lazy, per-line, per-row-validated iterator over one JSONL stream file.
///
/// Every row is validated as it is produced: version, then run ID, then vehicle identity. Nothing
/// beyond the current line is read ahead of time. The iterator is fused: after a row-level error,
/// it yields `None` without reading later rows.
pub struct LineRecords<T> {
    lines: Lines<BufReader<File>>,
    path: PathBuf,
    expected_run_id: RunId,
    expected_vehicle: String,
    expected_session_started_at: UnixTimestampV1,
    expected_schema_version: u32,
    line: usize,
    finished: bool,
    _payload: PhantomData<T>,
}

/// Internal payload contract used by stream iterators to enforce manifest session identity.
pub trait HasSessionStartedAt {
    fn session_started_at(&self) -> UnixTimestampV1;
}

impl HasSessionStartedAt for DiagnosticPayloadV1 {
    fn session_started_at(&self) -> UnixTimestampV1 {
        self.session_started_at
    }
}

impl HasSessionStartedAt for LedgerPayloadV1 {
    fn session_started_at(&self) -> UnixTimestampV1 {
        self.session_started_at
    }
}

pub type DiagnosticRecords = LineRecords<DiagnosticPayloadV1>;
pub type LedgerRecords = LineRecords<LedgerPayloadV1>;

impl<T> std::fmt::Debug for LineRecords<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LineRecords")
            .field("path", &self.path)
            .field("expected_run_id", &self.expected_run_id)
            .field("expected_vehicle", &self.expected_vehicle)
            .field(
                "expected_session_started_at",
                &self.expected_session_started_at,
            )
            .field("line", &self.line)
            .field("expected_schema_version", &self.expected_schema_version)
            .field("finished", &self.finished)
            .finish()
    }
}

impl<T: DeserializeOwned + HasSessionStartedAt> Iterator for LineRecords<T> {
    type Item = Result<StreamEnvelopeV1<T>, ObservationError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let raw = self.lines.next()?;
        self.line += 1;
        let item = self.validate_line(raw);
        if item.is_err() {
            self.finished = true;
        }
        Some(item)
    }
}

impl<T: DeserializeOwned + HasSessionStartedAt> std::iter::FusedIterator for LineRecords<T> {}

impl<T: DeserializeOwned + HasSessionStartedAt> LineRecords<T> {
    fn validate_line(
        &self,
        raw: std::io::Result<String>,
    ) -> Result<StreamEnvelopeV1<T>, ObservationError> {
        let raw = raw.map_err(|source| ObservationError::Io {
            path: self.path.clone(),
            source,
        })?;

        let value: serde_json::Value =
            serde_json::from_str(&raw).map_err(|source| ObservationError::InvalidRecord {
                stream: self.path.clone(),
                line: self.line,
                message: source.to_string(),
            })?;

        let found_version_u64 = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ObservationError::InvalidRecord {
                stream: self.path.clone(),
                line: self.line,
                message: "missing schema_version".to_string(),
            })?;
        let found_version =
            u32::try_from(found_version_u64).map_err(|_| ObservationError::InvalidRecord {
                stream: self.path.clone(),
                line: self.line,
                message: format!("schema_version {found_version_u64} is out of range for u32"),
            })?;
        if !is_supported_schema_version(found_version) {
            return Err(ObservationError::UnsupportedSchema {
                found: found_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        if found_version != self.expected_schema_version {
            return Err(ObservationError::SchemaVersionMismatch {
                manifest: self.expected_schema_version,
                row: found_version,
            });
        }

        let envelope: StreamEnvelopeV1<T> =
            serde_json::from_value(value).map_err(|source| ObservationError::InvalidRecord {
                stream: self.path.clone(),
                line: self.line,
                message: source.to_string(),
            })?;

        if envelope.run_id != self.expected_run_id {
            return Err(ObservationError::RunIdMismatch {
                expected: self.expected_run_id.to_string(),
                found: envelope.run_id.to_string(),
            });
        }
        if envelope.vehicle_identity != self.expected_vehicle {
            return Err(ObservationError::VehicleMismatch {
                expected: self.expected_vehicle.clone(),
                found: envelope.vehicle_identity.clone(),
            });
        }
        let found_session = envelope.payload.session_started_at();
        if found_session != self.expected_session_started_at {
            return Err(ObservationError::SessionMismatch {
                expected: self.expected_session_started_at.to_string(),
                found: found_session.to_string(),
            });
        }

        Ok(envelope)
    }
}

impl RunReader {
    /// Validate the manifest and open a reader for `run_dir`.
    ///
    /// The manifest's `schema_version` is checked before any attempt to deserialize it as
    /// `ManifestV1`, and the declared stream filenames are checked to be exactly
    /// `diagnostic.jsonl` and `ledger.jsonl` before any stream file is opened.
    pub fn open(run_dir: impl AsRef<Path>) -> Result<Self, ObservationError> {
        let run_dir = run_dir.as_ref().to_path_buf();
        let manifest = read_manifest(&run_dir.join(MANIFEST_FILE_NAME))?;
        Ok(Self { run_dir, manifest })
    }

    pub fn manifest(&self) -> &ManifestV1 {
        &self.manifest
    }

    pub fn diagnostics(&self) -> Result<DiagnosticRecords, ObservationError> {
        self.open_stream(&self.manifest.streams.diagnostic)
    }

    pub fn ledger(&self) -> Result<LedgerRecords, ObservationError> {
        self.open_stream(&self.manifest.streams.ledger)
    }

    /// Eagerly collect every diagnostic and ledger row into a complete, in-memory `StoredRun`.
    pub fn load(&self) -> Result<StoredRun, ObservationError> {
        let diagnostics = self.diagnostics()?.collect::<Result<Vec<_>, _>>()?;
        let ledger = self.ledger()?.collect::<Result<Vec<_>, _>>()?;
        Ok(StoredRun {
            manifest: self.manifest.clone(),
            diagnostics,
            ledger,
        })
    }

    fn open_stream<T>(&self, file_name: &str) -> Result<LineRecords<T>, ObservationError> {
        let path = self.run_dir.join(file_name);
        let file = File::open(&path).map_err(|source| ObservationError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(LineRecords {
            lines: BufReader::new(file).lines(),
            path,
            expected_run_id: self.manifest.run_id.clone(),
            expected_vehicle: self.manifest.vehicle.identity.clone(),
            expected_session_started_at: self.manifest.session_started_at,
            expected_schema_version: self.manifest.schema_version,
            line: 0,
            finished: false,
            _payload: PhantomData,
        })
    }
}

fn read_manifest(path: &Path) -> Result<ManifestV1, ObservationError> {
    let text = fs::read_to_string(path).map_err(|source| ObservationError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|source| ObservationError::Json {
            path: path.to_path_buf(),
            source,
        })?;

    let found_version_u64 = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ObservationError::InvalidManifest {
            message: "missing schema_version".to_string(),
        })?;
    let found_version =
        u32::try_from(found_version_u64).map_err(|_| ObservationError::InvalidManifest {
            message: format!("schema_version {found_version_u64} is out of range for u32"),
        })?;
    if !is_supported_schema_version(found_version) {
        return Err(ObservationError::UnsupportedSchema {
            found: found_version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    let manifest: ManifestV1 =
        serde_json::from_value(value).map_err(|source| ObservationError::Json {
            path: path.to_path_buf(),
            source,
        })?;

    validate_stream_names(&manifest.streams)?;
    Ok(manifest)
}

/// Reject any manifest stream filename other than the exact expected literal. This is the only
/// check performed, so absolute paths and traversal segments such as `../diagnostic.jsonl` are
/// rejected as a side effect of not being an exact match rather than through separate path logic.
fn validate_stream_names(streams: &StreamsV1) -> Result<(), ObservationError> {
    if streams.diagnostic != DIAGNOSTIC_FILE_NAME {
        return Err(ObservationError::InvalidManifest {
            message: format!(
                "manifest diagnostic stream filename must be exactly {DIAGNOSTIC_FILE_NAME:?}, found {:?}",
                streams.diagnostic
            ),
        });
    }
    if streams.ledger != LEDGER_FILE_NAME {
        return Err(ObservationError::InvalidManifest {
            message: format!(
                "manifest ledger stream filename must be exactly {LEDGER_FILE_NAME:?}, found {:?}",
                streams.ledger
            ),
        });
    }
    Ok(())
}
