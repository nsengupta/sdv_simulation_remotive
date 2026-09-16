use std::time::Duration;

use common::facade::{
    DiagnosticKind, DiagnosticLevel, DiagnosticRecord, PublishedBcmContext, PublishedBcmState,
    PublishedDomainAction, PublishedFsmEvent, PublishedFsmState, PublishedHeadlampContext,
    PublishedHeadlampState, PublishedHealthContext, PublishedObservedBool,
    PublishedPowertrainContext, PublishedSccmContext, PublishedTransitionRecord,
    PublishedVehicleContext, PublishedVisibilityContext, PublishedWeatherContext,
    PublishedWheelRpm, PublishedWiperContext, PublishedWiperState, UnixTimestamp,
};

pub const RUN_ID: &str = "00000000-0000-4000-8000-000000000001";
pub const VEHICLE: &str = "test-vehicle";
pub const SESSION_SECONDS: u64 = 1_784_260_800;
pub const CREATED_SECONDS: u64 = 1_784_260_800;

fn ts(seconds: u64, nanosecond: u32) -> UnixTimestamp {
    UnixTimestamp::from_duration_since_epoch(Duration::new(seconds, nanosecond))
}

fn stored_ts(seconds: u64, nanosecond: u32) -> observation::UnixTimestampV1 {
    observation::UnixTimestampV1::new(seconds, nanosecond).unwrap()
}

pub fn fixed_run_metadata() -> observation::RunMetadata {
    observation::RunMetadata::new(
        observation::RunId::parse(RUN_ID).unwrap(),
        stored_ts(CREATED_SECONDS, 0),
        stored_ts(SESSION_SECONDS, 0),
        VEHICLE,
        None,
    )
}

pub fn sample_diagnostic() -> DiagnosticRecord {
    DiagnosticRecord {
        level: DiagnosticLevel::Warning,
        source: "VirtualCarActor",
        kind: DiagnosticKind::Text {
            text: "fixed warning".into(),
        },
        session_started_at: ts(SESSION_SECONDS, 0),
        recorded_at: ts(SESSION_SECONDS + 1, 0),
    }
}

pub fn sample_ledger() -> PublishedTransitionRecord {
    let context = PublishedVehicleContext {
        sccm: PublishedSccmContext {
            hazard_button_on: PublishedObservedBool::Unknown,
            hazard_mode_on: PublishedObservedBool::Off,
        },
        bcm: PublishedBcmContext {
            state: PublishedBcmState::Ready,
            left_turn_request_on: PublishedObservedBool::Unknown,
            right_turn_request_on: PublishedObservedBool::Unknown,
        },
        powertrain: PublishedPowertrainContext {
            wheel_rpm: PublishedWheelRpm {
                front_left: 1,
                front_right: 2,
                rear_left: 3,
                rear_right: 4,
            },
            speed_kph: 42,
        },
        health: PublishedHealthContext {
            fuel_level_pct: 75,
            oil_pressure_kpa: 90,
            tyre_pressure_ok: true,
        },
        visibility: PublishedVisibilityContext { ambient_lux: 20 },
        weather: PublishedWeatherContext { raining: false },
        headlamp: PublishedHeadlampContext {
            state: PublishedHeadlampState::OnRequested,
            ack_pending_since: Some(ts(SESSION_SECONDS, 750_000_000)),
        },
        wiper: PublishedWiperContext {
            state: PublishedWiperState::Off,
        },
    };

    PublishedTransitionRecord {
        car_identity: VEHICLE.into(),
        session_started_at: ts(SESSION_SECONDS, 0),
        record_seq: 7,
        recorded_at: ts(SESSION_SECONDS + 2, 0),
        event: PublishedFsmEvent::UpdateRpm(1500),
        old_state: PublishedFsmState::ExtremeOperationWarning {
            entered_at: ts(SESSION_SECONDS, 500_000_000),
        },
        next_state: PublishedFsmState::Driving,
        old_ctx: context,
        current_ctx: context,
        actions: vec![PublishedDomainAction::LogWarning("fixed warning".into())],
    }
}
