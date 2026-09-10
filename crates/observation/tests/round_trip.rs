mod support;

use std::time::Duration;

use common::facade::UnixTimestamp;
use observation::schema::v1::{diagnostic_envelope, ledger_envelope};
use observation::{ObservationError, RunReader, RunWriter, UnixTimestampV1};
use support::{
    CREATED_SECONDS, RUN_ID, SESSION_SECONDS, VEHICLE, fixed_run_metadata, sample_diagnostic,
    sample_ledger,
};

fn numeric_ts(seconds: u64, nanosecond: u32) -> serde_json::Value {
    serde_json::json!({
        "unix_seconds": seconds,
        "nanosecond": nanosecond,
    })
}

#[test]
fn unix_timestamp_v1_splits_and_round_trips_live_values() {
    let live = UnixTimestamp::from_duration_since_epoch(Duration::new(1_752_724_801, 120_000_000));
    let stored = UnixTimestampV1::from_live(live);
    assert_eq!(
        serde_json::to_value(stored).unwrap(),
        numeric_ts(1_752_724_801, 120_000_000)
    );
    assert_eq!(stored.to_live(), live);
    assert_eq!(
        UnixTimestampV1::new(1_752_724_801, 999_999_999)
            .unwrap()
            .nanosecond,
        999_999_999
    );
}

#[test]
fn invalid_nanosecond_is_rejected() {
    let error = UnixTimestampV1::new(1, 1_000_000_000).unwrap_err();
    match error {
        ObservationError::InvalidTimestamp { reason, .. } => {
            assert!(reason.contains("nanosecond"), "{reason}");
        }
        other => panic!("expected InvalidTimestamp, got {other:?}"),
    }

    let err = serde_json::from_value::<UnixTimestampV1>(serde_json::json!({
        "unix_seconds": 1,
        "nanosecond": 1_000_000_000_u32
    }))
    .unwrap_err();
    assert!(err.to_string().contains("nanosecond"), "{err}");
}

#[test]
fn unix_timestamp_v1_orders_across_a_second_boundary() {
    let earlier = UnixTimestampV1::new(100, 999_999_999).unwrap();
    let later = UnixTimestampV1::new(101, 0).unwrap();
    assert!(earlier < later);
}

#[test]
fn unix_timestamp_v1_display_uses_readable_utc_layout() {
    let stamp = UnixTimestampV1::new(SESSION_SECONDS, 653_061_112).unwrap();
    assert_eq!(stamp.to_string(), "2026-07-17 | 04:00:00:653061112 (UTC)");
}

#[test]
fn diagnostic_projection_uses_snake_case_level_and_numeric_timestamp() {
    let entry = diagnostic_envelope(&fixed_run_metadata(), &sample_diagnostic()).unwrap();
    let json = serde_json::to_value(entry).unwrap();
    assert_eq!(json["schema_version"], 4);
    assert_eq!(json["payload"]["level"], "warning");
    assert_eq!(json["payload"]["kind"]["type"], "text");
    assert_eq!(json["payload"]["kind"]["text"], "fixed warning");
    assert_eq!(json["vehicle_identity"], VEHICLE);
    assert_eq!(json["recorded_at"], numeric_ts(SESSION_SECONDS + 1, 0));
    assert!(json["recorded_at"]["unix_seconds"].is_number());
    assert!(json["recorded_at"]["nanosecond"].is_number());
    assert!(json["recorded_at"]["unix_seconds"].as_str().is_none());
}

#[test]
fn ledger_projection_is_lossless_and_explicitly_tagged() {
    let entry = ledger_envelope(&fixed_run_metadata(), &sample_ledger()).unwrap();
    let json = serde_json::to_value(entry).unwrap();
    assert_eq!(json["schema_version"], 4);
    assert_eq!(
        json["payload"]["session_started_at"],
        numeric_ts(SESSION_SECONDS, 0)
    );
    assert_eq!(json["payload"]["record_seq"], 7);
    assert_eq!(json["payload"]["event"]["type"], "update_rpm");
    assert_eq!(json["payload"]["event"]["rpm"], 1500);
    assert_eq!(
        json["payload"]["old_state"]["type"],
        "extreme_operation_warning"
    );
    assert_eq!(
        json["payload"]["old_state"]["entered_at"],
        numeric_ts(SESSION_SECONDS, 500_000_000)
    );
    assert_eq!(json["payload"]["next_state"]["type"], "driving");
    assert_eq!(json["payload"]["old_ctx"]["powertrain"]["speed_kph"], 42);
    assert_eq!(
        json["payload"]["old_ctx"]["powertrain"]["wheel_rpm"]["front_left"],
        1
    );
    assert_eq!(
        json["payload"]["old_ctx"]["powertrain"]["wheel_rpm"]["front_right"],
        2
    );
    assert_eq!(
        json["payload"]["old_ctx"]["powertrain"]["wheel_rpm"]["rear_left"],
        3
    );
    assert_eq!(
        json["payload"]["old_ctx"]["powertrain"]["wheel_rpm"]["rear_right"],
        4
    );
    assert_eq!(json["payload"]["old_ctx"]["health"]["fuel_level_pct"], 75);
    assert_eq!(json["payload"]["old_ctx"]["health"]["oil_pressure_kpa"], 90);
    assert_eq!(
        json["payload"]["old_ctx"]["health"]["tyre_pressure_ok"],
        true
    );
    assert_eq!(
        json["payload"]["current_ctx"]["visibility"]["ambient_lux"],
        20
    );
    assert_eq!(
        json["payload"]["current_ctx"]["headlamp"]["state"],
        "on_requested"
    );
    assert_eq!(
        json["payload"]["current_ctx"]["headlamp"]["ack_pending_since"],
        numeric_ts(SESSION_SECONDS, 750_000_000)
    );
    assert_eq!(json["payload"]["actions"][0]["type"], "log_warning");
    assert_eq!(json["payload"]["actions"][0]["message"], "fixed warning");
}

#[test]
fn ledger_projection_rejects_vehicle_mismatch() {
    let mut record = sample_ledger();
    record.car_identity = "other-vehicle".into();

    let error = ledger_envelope(&fixed_run_metadata(), &record).unwrap_err();
    match error {
        ObservationError::VehicleMismatch { expected, found } => {
            assert_eq!(expected, VEHICLE);
            assert_eq!(found, "other-vehicle");
        }
        other => panic!("expected VehicleMismatch, got {other:?}"),
    }
}

#[test]
fn projection_rejects_session_mismatch() {
    let mut record = sample_diagnostic();
    record.session_started_at =
        UnixTimestamp::from_duration_since_epoch(Duration::new(CREATED_SECONDS + 9, 0));
    let error = diagnostic_envelope(&fixed_run_metadata(), &record).unwrap_err();
    match error {
        ObservationError::SessionMismatch { .. } => {}
        other => panic!("expected SessionMismatch, got {other:?}"),
    }
}

#[test]
fn writer_creates_run_directory_and_writes_all_stream_files() {
    let temp = tempfile::tempdir().unwrap();
    let metadata = fixed_run_metadata();
    let mut writer = RunWriter::create(temp.path(), metadata.clone()).unwrap();
    assert_eq!(writer.run_dir(), temp.path().join(RUN_ID));

    writer.record_diagnostic(&sample_diagnostic()).unwrap();
    writer.record_ledger(&sample_ledger()).unwrap();
    writer.finish().unwrap();

    let run_dir = temp.path().join(RUN_ID);
    assert!(run_dir.join("manifest.json").is_file());
    assert!(run_dir.join("diagnostic.jsonl").is_file());
    assert!(run_dir.join("ledger.jsonl").is_file());

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["created_at"], numeric_ts(CREATED_SECONDS, 0));
    assert_eq!(
        manifest["session_started_at"],
        numeric_ts(SESSION_SECONDS, 0)
    );
}

#[test]
fn writer_rejects_a_run_directory_that_already_exists() {
    let temp = tempfile::tempdir().unwrap();
    let metadata = fixed_run_metadata();
    let _first = RunWriter::create(temp.path(), metadata.clone()).unwrap();

    let error = RunWriter::create(temp.path(), metadata).unwrap_err();
    assert!(
        error.to_string().contains("run directory already exists"),
        "unexpected error text: {error}"
    );
}

#[test]
fn reader_streams_and_loads_multiple_rows() {
    let temp = tempfile::tempdir().unwrap();
    let metadata = fixed_run_metadata();
    let mut writer = RunWriter::create(temp.path(), metadata.clone()).unwrap();
    writer.record_diagnostic(&sample_diagnostic()).unwrap();
    writer.record_diagnostic(&sample_diagnostic()).unwrap();
    writer.record_ledger(&sample_ledger()).unwrap();
    writer.record_ledger(&sample_ledger()).unwrap();
    writer.finish().unwrap();

    let run_dir = temp.path().join(RUN_ID);
    let reader = RunReader::open(&run_dir).unwrap();
    let diagnostics = reader
        .diagnostics()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let ledger = reader
        .ledger()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(ledger.len(), 2);

    let stored = reader.load().unwrap();
    assert_eq!(stored.diagnostics, diagnostics);
    assert_eq!(stored.ledger, ledger);
    assert_eq!(stored.manifest.scenario, metadata.scenario);
}

#[test]
fn reader_rejects_a_run_directory_missing_a_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let error = RunReader::open(temp.path()).unwrap_err();
    match error {
        ObservationError::Io { path, .. } => {
            assert!(path.ends_with("manifest.json"), "path was {path:?}");
        }
        other => panic!("expected Io, got {other:?}"),
    }
}
