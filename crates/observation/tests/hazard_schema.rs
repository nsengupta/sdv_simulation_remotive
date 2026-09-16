#[allow(dead_code)]
mod support;

use common::facade::{
    DiagnosticKind, PublishedDomainAction, PublishedFsmEvent, PublishedObservedBool,
};
use observation::schema::CURRENT_SCHEMA_VERSION;
use observation::schema::v1::{
    BcmContextV1, BcmStateV1, DiagnosticKindV1, DomainActionV1, FlcmContextV1, FsmEventV1,
    ObservedBoolV1, SccmContextV1, diagnostic_envelope, ledger_envelope,
};

#[test]
fn current_schema_version_is_v7() {
    assert_eq!(CURRENT_SCHEMA_VERSION, 7);
}

#[test]
fn v5_sccm_json_without_hazard_mode_defaults_off() {
    let sccm: SccmContextV1 = serde_json::from_str(r#"{"hazard_button_on":"on"}"#).unwrap();
    assert_eq!(sccm.hazard_mode_on, ObservedBoolV1::Off);
}

#[test]
fn v7_initial_observed_values_serialize_as_unknown() {
    let metadata = support::fixed_run_metadata();
    let live = support::sample_ledger();
    let envelope = ledger_envelope(&metadata, &live).expect("sample ledger projection");
    let json = serde_json::to_value(&envelope).expect("serialize v5 envelope");

    assert_eq!(json["schema_version"], 7);
    assert_eq!(
        json["payload"]["old_ctx"]["sccm"]["hazard_button_on"],
        "unknown"
    );
    assert_eq!(
        json["payload"]["current_ctx"]["sccm"]["hazard_button_on"],
        "unknown"
    );
    assert_eq!(
        json["payload"]["current_ctx"]["bcm"]["left_turn_request_on"],
        "unknown"
    );
    assert_eq!(
        json["payload"]["current_ctx"]["bcm"]["right_turn_request_on"],
        "unknown"
    );
    assert_eq!(
        json["payload"]["current_ctx"]["flcm"],
        serde_json::json!({
            "left_low_beam_status_ok": "unknown",
            "right_low_beam_status_ok": "unknown",
            "silent": false
        })
    );
}

#[test]
fn missing_flcm_context_defaults_to_unknown_and_not_silent() {
    let flcm: FlcmContextV1 = serde_json::from_str("{}").unwrap();
    assert_eq!(flcm.left_low_beam_status_ok, ObservedBoolV1::Unknown);
    assert_eq!(flcm.right_low_beam_status_ok, ObservedBoolV1::Unknown);
    assert!(!flcm.silent);
}

#[test]
fn flcm_fault_diagnostic_has_a_dedicated_wire_variant() {
    let mut diagnostic = support::sample_diagnostic();
    diagnostic.kind = DiagnosticKind::FlcmLampFault {
        silent: true,
        left_fail: false,
        right_fail: true,
    };

    let envelope = diagnostic_envelope(&support::fixed_run_metadata(), &diagnostic).unwrap();
    assert_eq!(
        envelope.payload.kind,
        DiagnosticKindV1::FlcmLampFault {
            silent: true,
            left_fail: false,
            right_fail: true,
        }
    );
}

#[test]
fn observed_events_round_trip_losslessly() {
    let metadata = support::fixed_run_metadata();
    let cases = [
        (
            PublishedFsmEvent::HazardButtonObserved(true),
            FsmEventV1::HazardButtonObserved { pressed: true },
        ),
        (
            PublishedFsmEvent::LeftTurnRequestObserved(true),
            FsmEventV1::LeftTurnRequestObserved { pressed: true },
        ),
        (
            PublishedFsmEvent::RightTurnRequestObserved(false),
            FsmEventV1::RightTurnRequestObserved { pressed: false },
        ),
    ];

    for (live_event, expected_dto) in cases {
        let mut live = support::sample_ledger();
        live.event = live_event.clone();
        live.current_ctx.sccm.hazard_button_on = PublishedObservedBool::On;
        live.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        live.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::Off;

        let envelope = ledger_envelope(&metadata, &live).expect("observed ledger projection");
        assert_eq!(envelope.schema_version, 7);
        assert_eq!(envelope.payload.event, expected_dto);
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
            ObservedBoolV1::Off
        );

        let restored = observation::ledger_from_envelope(&envelope).expect("observed round trip");
        assert_eq!(restored, live);
    }
}

#[test]
fn schema_v4_hazard_button_changed_and_set_turn_lights_remain_readable() {
    let event: FsmEventV1 =
        serde_json::from_str(r#"{"type":"hazard_button_changed","pressed":true}"#)
            .expect("historical HazardButtonChanged");
    assert_eq!(event, FsmEventV1::HazardButtonChanged { pressed: true });

    let action: DomainActionV1 =
        serde_json::from_str(r#"{"type":"set_turn_lights","left_on":true,"right_on":true}"#)
            .expect("historical SetTurnLights");
    assert_eq!(
        action,
        DomainActionV1::SetTurnLights {
            left_on: true,
            right_on: true,
        }
    );

    let sccm: SccmContextV1 =
        serde_json::from_str(r#"{"hazard_button_on":true}"#).expect("v4 boolean SCCM");
    assert_eq!(sccm.hazard_button_on, ObservedBoolV1::On);

    let bcm: BcmContextV1 = serde_json::from_str(
        r#"{"state":"ready","left_turn_request_on":false,"right_turn_request_on":true}"#,
    )
    .expect("v4 boolean BCM");
    assert_eq!(bcm.state, BcmStateV1::Ready);
    assert_eq!(bcm.left_turn_request_on, ObservedBoolV1::Off);
    assert_eq!(bcm.right_turn_request_on, ObservedBoolV1::On);
}

#[test]
fn schema_v1_round_trips_hazard_context_and_atomic_action() {
    let metadata = support::fixed_run_metadata();
    let mut live = support::sample_ledger();
    live.event = PublishedFsmEvent::HazardButtonChanged(true);
    live.current_ctx.sccm.hazard_button_on = PublishedObservedBool::On;
    live.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
    live.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;
    live.actions = vec![PublishedDomainAction::SetTurnLights {
        left_on: true,
        right_on: true,
    }];

    let envelope = ledger_envelope(&metadata, &live).expect("hazard ledger projection");
    assert_eq!(
        envelope.payload.event,
        FsmEventV1::HazardButtonChanged { pressed: true }
    );
    assert_eq!(
        envelope.payload.current_ctx.sccm.hazard_button_on,
        ObservedBoolV1::On
    );
    assert_eq!(envelope.payload.current_ctx.bcm.state, BcmStateV1::Ready);
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

    let restored = observation::ledger_from_envelope(&envelope).expect("hazard round trip");
    assert_eq!(restored, live);
}
