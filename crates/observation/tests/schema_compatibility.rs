mod support;

use std::path::{Path, PathBuf};

use observation::{ObservationError, RunReader, RunWriter};
use support::{RUN_ID, VEHICLE, fixed_run_metadata, sample_diagnostic, sample_ledger};

#[test]
fn pre_phase_i_v3_ledger_defaults_missing_sccm_and_bcm_contexts() {
    let fixture = include_str!("fixtures/pre_phase_i_v3_ledger.json");
    let envelope: observation::schema::v1::StreamEnvelopeV1<
        observation::schema::v1::LedgerPayloadV1,
    > = serde_json::from_str(fixture).expect("pre-Phase-I schema-v3 ledger must deserialize");

    for context in [&envelope.payload.old_ctx, &envelope.payload.current_ctx] {
        assert!(!context.sccm.hazard_button_on);
        assert_eq!(context.bcm.state, observation::schema::v1::BcmStateV1::Off);
        assert!(!context.bcm.left_turn_request_on);
        assert!(!context.bcm.right_turn_request_on);
    }
}

/// Write a fresh, valid fixture (two diagnostics, one ledger row) under `parent` and return the
/// created run directory. Each test starts from this valid baseline and independently mutates
/// exactly one aspect of it, per the Task 4 compatibility matrix.
fn write_fixture(parent: &Path) -> PathBuf {
    let metadata = fixed_run_metadata();
    let mut writer = RunWriter::create(parent, metadata).unwrap();
    writer.record_diagnostic(&sample_diagnostic()).unwrap();
    writer.record_diagnostic(&sample_diagnostic()).unwrap();
    writer.record_ledger(&sample_ledger()).unwrap();
    writer.finish().unwrap();
    parent.join(RUN_ID)
}

fn mutate_json_file(path: &Path, mutate: impl FnOnce(&mut serde_json::Value)) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    mutate(&mut value);
    std::fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&value).unwrap()),
    )
    .unwrap();
}

fn mutate_jsonl_line(path: &Path, line_number: usize, mutate: impl FnOnce(&mut serde_json::Value)) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let index = line_number - 1;
    let mut value: serde_json::Value = serde_json::from_str(&lines[index]).unwrap();
    mutate(&mut value);
    lines[index] = serde_json::to_string(&value).unwrap();
    std::fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
}

fn overwrite_jsonl_line(path: &Path, line_number: usize, replacement: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines[line_number - 1] = replacement.to_string();
    std::fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
}

#[test]
fn manifest_schema_version_mismatch_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_json_file(&run_dir.join("manifest.json"), |value| {
        value["schema_version"] = serde_json::json!(4);
    });

    let error = RunReader::open(&run_dir).unwrap_err();
    match error {
        ObservationError::UnsupportedSchema { found, supported } => {
            assert_eq!(found, 4);
            assert_eq!(supported, 3);
        }
        other => panic!("expected UnsupportedSchema, got {other:?}"),
    }
}

#[test]
fn oversized_manifest_schema_version_is_rejected_as_invalid_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_json_file(&run_dir.join("manifest.json"), |value| {
        value["schema_version"] = serde_json::json!(u64::MAX);
    });

    let error = RunReader::open(&run_dir).unwrap_err();
    match error {
        ObservationError::InvalidManifest { message } => {
            assert!(message.contains(&u64::MAX.to_string()), "{message}");
            assert!(message.contains("out of range"), "{message}");
            assert!(message.contains("u32"), "{message}");
        }
        other => panic!("expected InvalidManifest, got {other:?}"),
    }
}

#[test]
fn diagnostic_row_schema_version_mismatch_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_jsonl_line(&run_dir.join("diagnostic.jsonl"), 1, |value| {
        value["schema_version"] = serde_json::json!(4);
    });

    let reader = RunReader::open(&run_dir).unwrap();
    let error = reader.diagnostics().unwrap().next().unwrap().unwrap_err();
    match error {
        ObservationError::UnsupportedSchema { found, supported } => {
            assert_eq!(found, 4);
            assert_eq!(supported, 3);
        }
        other => panic!("expected UnsupportedSchema, got {other:?}"),
    }
}

#[test]
fn oversized_diagnostic_row_schema_version_reports_stream_and_line() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_jsonl_line(&run_dir.join("diagnostic.jsonl"), 1, |value| {
        value["schema_version"] = serde_json::json!(u64::MAX);
    });

    let reader = RunReader::open(&run_dir).unwrap();
    let error = reader.diagnostics().unwrap().next().unwrap().unwrap_err();
    match error {
        ObservationError::InvalidRecord {
            stream,
            line,
            message,
        } => {
            assert_eq!(line, 1);
            assert!(stream.ends_with("diagnostic.jsonl"), "{stream:?}");
            assert!(message.contains(&u64::MAX.to_string()), "{message}");
            assert!(message.contains("out of range"), "{message}");
            assert!(message.contains("u32"), "{message}");
        }
        other => panic!("expected InvalidRecord, got {other:?}"),
    }
}

#[test]
fn diagnostic_row_run_id_mismatch_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_jsonl_line(&run_dir.join("diagnostic.jsonl"), 1, |value| {
        value["run_id"] = serde_json::json!("00000000-0000-4000-8000-000000000099");
    });

    let reader = RunReader::open(&run_dir).unwrap();
    let error = reader.diagnostics().unwrap().next().unwrap().unwrap_err();
    match error {
        ObservationError::RunIdMismatch { expected, found } => {
            assert_eq!(expected, RUN_ID);
            assert_eq!(found, "00000000-0000-4000-8000-000000000099");
        }
        other => panic!("expected RunIdMismatch, got {other:?}"),
    }
}

#[test]
fn diagnostic_row_vehicle_identity_mismatch_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_jsonl_line(&run_dir.join("diagnostic.jsonl"), 1, |value| {
        value["vehicle_identity"] = serde_json::json!("other-vehicle");
    });

    let reader = RunReader::open(&run_dir).unwrap();
    let error = reader.diagnostics().unwrap().next().unwrap().unwrap_err();
    match error {
        ObservationError::VehicleMismatch { expected, found } => {
            assert_eq!(expected, VEHICLE);
            assert_eq!(found, "other-vehicle");
        }
        other => panic!("expected VehicleMismatch, got {other:?}"),
    }
}

#[test]
fn diagnostic_row_session_started_at_mismatch_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_jsonl_line(&run_dir.join("diagnostic.jsonl"), 1, |value| {
        value["payload"]["session_started_at"] = serde_json::json!({
            "unix_seconds": 1,
            "nanosecond": 0
        });
    });

    let reader = RunReader::open(&run_dir).unwrap();
    let error = reader.diagnostics().unwrap().next().unwrap().unwrap_err();
    match error {
        ObservationError::SessionMismatch { .. } => {}
        other => panic!("expected SessionMismatch, got {other:?}"),
    }
}

#[test]
fn malformed_json_on_diagnostic_line_two_reports_stream_and_line() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    overwrite_jsonl_line(&run_dir.join("diagnostic.jsonl"), 2, "{not valid json");

    let reader = RunReader::open(&run_dir).unwrap();
    let mut records = reader.diagnostics().unwrap();
    records.next().unwrap().unwrap();
    let error = records.next().unwrap().unwrap_err();

    let message = error.to_string();
    match error {
        ObservationError::InvalidRecord { stream, line, .. } => {
            assert_eq!(line, 2);
            assert!(
                stream.to_string_lossy().ends_with("diagnostic.jsonl"),
                "unexpected stream path: {stream:?}"
            );
        }
        other => panic!("expected InvalidRecord, got {other:?}"),
    }
    assert!(message.contains("diagnostic.jsonl"), "{message}");
    assert!(message.contains("line 2"), "{message}");
}

#[test]
fn line_records_stop_after_the_first_row_error() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    let diagnostic_path = run_dir.join("diagnostic.jsonl");
    let original = std::fs::read_to_string(&diagnostic_path).unwrap();
    let first_row = original.lines().next().unwrap();
    std::fs::write(
        &diagnostic_path,
        format!("{first_row}\n{{not valid json\n{first_row}\n"),
    )
    .unwrap();

    let reader = RunReader::open(&run_dir).unwrap();
    let mut records = reader.diagnostics().unwrap();
    records.next().unwrap().unwrap();
    assert!(records.next().unwrap().is_err());
    assert!(records.next().is_none());
}

#[test]
fn invalid_row_timestamp_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_jsonl_line(&run_dir.join("diagnostic.jsonl"), 1, |value| {
        value["recorded_at"] = serde_json::json!({
            "unix_seconds": 1,
            "nanosecond": 1_000_000_000_u32
        });
    });

    let reader = RunReader::open(&run_dir).unwrap();
    let error = reader.diagnostics().unwrap().next().unwrap().unwrap_err();
    match error {
        ObservationError::InvalidRecord {
            stream,
            line,
            message,
        } => {
            assert_eq!(line, 1);
            assert!(stream.to_string_lossy().ends_with("diagnostic.jsonl"));
            assert!(message.contains("nanosecond"), "{message}");
        }
        other => panic!("expected InvalidRecord, got {other:?}"),
    }
}

#[test]
fn manifest_stream_traversal_filename_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_json_file(&run_dir.join("manifest.json"), |value| {
        value["streams"]["diagnostic"] = serde_json::json!("../diagnostic.jsonl");
    });

    let error = RunReader::open(&run_dir).unwrap_err();
    match error {
        ObservationError::InvalidManifest { message } => {
            assert!(message.contains("diagnostic"), "{message}");
        }
        other => panic!("expected InvalidManifest, got {other:?}"),
    }
}

#[test]
fn missing_ledger_stream_file_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    std::fs::remove_file(run_dir.join("ledger.jsonl")).unwrap();

    let reader = RunReader::open(&run_dir).unwrap();
    let error = reader.ledger().unwrap_err();
    match error {
        ObservationError::Io { path, .. } => {
            assert!(path.ends_with("ledger.jsonl"), "path was {path:?}");
        }
        other => panic!("expected Io, got {other:?}"),
    }
}
