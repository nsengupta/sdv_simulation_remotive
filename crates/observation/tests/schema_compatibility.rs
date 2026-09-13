mod support;

use std::path::{Path, PathBuf};

use observation::schema::CURRENT_SCHEMA_VERSION;
use observation::schema::v1::{DomainActionV1, FsmEventV1, ObservedBoolV1};
use observation::{ObservationError, RunReader, RunWriter};
use support::{RUN_ID, VEHICLE, fixed_run_metadata, sample_diagnostic, sample_ledger};

#[test]
fn pre_phase_i_v3_run_reads_with_defaulted_sccm_and_bcm_contexts() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path().join(RUN_ID);
    std::fs::create_dir(&run_dir).unwrap();
    std::fs::write(
        run_dir.join("manifest.json"),
        format!(
            r#"{{
  "schema_version": 3,
  "run_id": "{RUN_ID}",
  "created_at": {{"unix_seconds": 1784260800, "nanosecond": 0}},
  "session_started_at": {{"unix_seconds": 1784260800, "nanosecond": 0}},
  "vehicle": {{"identity": "{VEHICLE}"}},
  "scenario": null,
  "streams": {{"diagnostic": "diagnostic.jsonl", "ledger": "ledger.jsonl"}}
}}"#
        ),
    )
    .unwrap();
    std::fs::write(run_dir.join("diagnostic.jsonl"), "").unwrap();
    let fixture = include_str!("fixtures/pre_phase_i_v3_ledger.json");
    let fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
    std::fs::write(
        run_dir.join("ledger.jsonl"),
        format!("{}\n", serde_json::to_string(&fixture).unwrap()),
    )
    .unwrap();
    let reader = RunReader::open(&run_dir).expect("schema-v3 manifest must remain supported");
    let envelope = reader
        .ledger()
        .unwrap()
        .next()
        .unwrap()
        .expect("pre-Phase-I schema-v3 ledger must deserialize");

    for context in [&envelope.payload.old_ctx, &envelope.payload.current_ctx] {
        assert_eq!(context.sccm.hazard_button_on, ObservedBoolV1::Off);
        assert_eq!(context.bcm.state, observation::schema::v1::BcmStateV1::Off);
        assert_eq!(context.bcm.left_turn_request_on, ObservedBoolV1::Off);
        assert_eq!(context.bcm.right_turn_request_on, ObservedBoolV1::Off);
    }
}

#[test]
fn current_schema_version_is_v5() {
    assert_eq!(CURRENT_SCHEMA_VERSION, 5);
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

#[test]
fn new_writer_emits_schema_v5_manifest_and_rows() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["schema_version"], 5);

    for stream in ["diagnostic.jsonl", "ledger.jsonl"] {
        let row: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(run_dir.join(stream))
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(row["schema_version"], 5, "{stream}");
    }
}

#[test]
fn historical_v1_v2_v3_golden_fixtures_still_read() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    let temp = tempfile::tempdir().unwrap();
    let v1_dir = temp.path().join(RUN_ID);
    std::fs::create_dir(&v1_dir).unwrap();
    std::fs::write(
        v1_dir.join("manifest.json"),
        format!(
            r#"{{
  "schema_version": 1,
  "run_id": "{RUN_ID}",
  "created_at": {{"unix_seconds": 1784260800, "nanosecond": 0}},
  "session_started_at": {{"unix_seconds": 1784260800, "nanosecond": 0}},
  "vehicle": {{"identity": "{VEHICLE}"}},
  "scenario": null,
  "streams": {{"diagnostic": "diagnostic.jsonl", "ledger": "ledger.jsonl"}}
}}"#
        ),
    )
    .unwrap();
    std::fs::write(v1_dir.join("diagnostic.jsonl"), "").unwrap();
    let fixture = include_str!("fixtures/pre_phase_i_v3_ledger.json");
    let mut fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
    fixture["schema_version"] = serde_json::json!(1);
    std::fs::write(
        v1_dir.join("ledger.jsonl"),
        format!("{}\n", serde_json::to_string(&fixture).unwrap()),
    )
    .unwrap();
    let v1 = RunReader::open(&v1_dir).expect("schema-v1 fixture must remain supported");
    let envelope = v1
        .ledger()
        .unwrap()
        .next()
        .unwrap()
        .expect("schema-v1 ledger must deserialize");
    assert_eq!(
        envelope.payload.current_ctx.sccm.hazard_button_on,
        ObservedBoolV1::Off
    );

    let v2 = crate_dir.join("testdata/golden/v2").join(RUN_ID);
    let v2_reader = RunReader::open(&v2).expect("schema-v2 golden must remain supported");
    let diagnostics = v2_reader
        .diagnostics()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .expect("schema-v2 diagnostics must deserialize");
    assert!(!diagnostics.is_empty());

    let v3 = crate_dir.join("testdata/golden/v3").join(RUN_ID);
    RunReader::open(&v3)
        .expect("schema-v3 golden must remain supported")
        .load()
        .expect("schema-v3 golden must load");
}

#[test]
fn v4_hazard_button_changed_and_set_turn_lights_remain_readable() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_json_file(&run_dir.join("manifest.json"), |value| {
        value["schema_version"] = serde_json::json!(4);
    });
    mutate_jsonl_line(&run_dir.join("diagnostic.jsonl"), 1, |value| {
        value["schema_version"] = serde_json::json!(4);
    });
    mutate_jsonl_line(&run_dir.join("diagnostic.jsonl"), 2, |value| {
        value["schema_version"] = serde_json::json!(4);
    });
    mutate_jsonl_line(&run_dir.join("ledger.jsonl"), 1, |value| {
        value["schema_version"] = serde_json::json!(4);
        value["payload"]["event"] = serde_json::json!({
            "type": "hazard_button_changed",
            "pressed": true
        });
        value["payload"]["current_ctx"]["sccm"]["hazard_button_on"] = serde_json::json!(true);
        value["payload"]["current_ctx"]["bcm"]["left_turn_request_on"] = serde_json::json!(true);
        value["payload"]["current_ctx"]["bcm"]["right_turn_request_on"] = serde_json::json!(true);
        value["payload"]["actions"] = serde_json::json!([{
            "type": "set_turn_lights",
            "left_on": true,
            "right_on": true
        }]);
    });

    let reader = RunReader::open(&run_dir).expect("schema-v4 historical run must remain supported");
    let envelope = reader
        .ledger()
        .unwrap()
        .next()
        .unwrap()
        .expect("v4 hazard ledger must deserialize");
    assert_eq!(
        envelope.payload.event,
        FsmEventV1::HazardButtonChanged { pressed: true }
    );
    assert_eq!(
        envelope.payload.current_ctx.sccm.hazard_button_on,
        ObservedBoolV1::On
    );
    assert_eq!(
        envelope.payload.current_ctx.bcm.left_turn_request_on,
        ObservedBoolV1::On
    );
    assert_eq!(
        envelope.payload.current_ctx.bcm.right_turn_request_on,
        ObservedBoolV1::On
    );
    assert_eq!(
        envelope.payload.actions,
        vec![DomainActionV1::SetTurnLights {
            left_on: true,
            right_on: true,
        }]
    );
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
fn unsupported_v6_manifest_is_rejected_clearly() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = write_fixture(temp.path());
    mutate_json_file(&run_dir.join("manifest.json"), |value| {
        value["schema_version"] = serde_json::json!(6);
    });

    let error = RunReader::open(&run_dir).unwrap_err();
    match error {
        ObservationError::UnsupportedSchema { found, supported } => {
            assert_eq!(found, 6);
            assert_eq!(supported, 5);
        }
        other => panic!("expected UnsupportedSchema, got {other:?}"),
    }
    assert!(
        error.to_string().contains('6') && error.to_string().contains('5'),
        "unsupported v6 must name the found and supported versions: {error}"
    );
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
        value["schema_version"] = serde_json::json!(3);
    });

    let reader = RunReader::open(&run_dir).unwrap();
    let error = reader.diagnostics().unwrap().next().unwrap().unwrap_err();
    match error {
        ObservationError::SchemaVersionMismatch { manifest, row } => {
            assert_eq!(manifest, 5);
            assert_eq!(row, 3);
        }
        other => panic!("expected SchemaVersionMismatch, got {other:?}"),
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
