use remotive_bridge::cli::{
    DEFAULT_BROKER_URL, DEFAULT_CAN_INTERFACE, DEFAULT_RPM_CLAMP, DEFAULT_TICK_MS, parse_args,
};
use remotive_bridge::dbc_signal_decoder::{decode_dbc_off_or_on, decode_dbc_ok_or_fail};
use remotive_bridge::source::{
    BrokerObservation, CLIENT_ID, FLCM_NAMESPACE, HAZARD_NAME, HAZARD_NAMESPACE,
    LEFT_LOW_BEAM_STATUS_NAME, LEFT_TURN_NAME, ObservationRejectCounters, ProfileRpmSource,
    RIGHT_LOW_BEAM_STATUS_NAME, RIGHT_TURN_NAME, RpmSource, TURN_NAMESPACE,
    decode_observation_signals, subscription_config, subscription_ready_status,
};
use remotivelabs_broker::generated::base::signal::Payload;
use remotivelabs_broker::generated::base::{NameSpace, Signal, SignalId, Signals};
use std::num::NonZeroUsize;
use std::time::Duration;

#[test]
fn subscription_config_contains_exactly_five_ordered_signal_ids() {
    let config = subscription_config();
    assert_eq!(config.client_id.unwrap().id, CLIENT_ID);
    let ids = config.signals.unwrap().signal_id;
    let identities: Vec<_> = ids
        .iter()
        .map(|id| {
            (
                id.namespace.as_ref().unwrap().name.as_str(),
                id.name.as_str(),
            )
        })
        .collect();
    assert_eq!(
        identities,
        [
            (HAZARD_NAMESPACE, HAZARD_NAME),
            (TURN_NAMESPACE, LEFT_TURN_NAME),
            (TURN_NAMESPACE, RIGHT_TURN_NAME),
            (FLCM_NAMESPACE, LEFT_LOW_BEAM_STATUS_NAME),
            (FLCM_NAMESPACE, RIGHT_LOW_BEAM_STATUS_NAME),
        ]
    );
    assert!(!config.on_change);
    assert!(!config.initial_empty);
}

#[test]
fn subscription_config_rejects_hello_world_signal_widening() {
    let config = subscription_config();
    let ids = config.signals.unwrap().signal_id;
    assert_eq!(
        ids.len(),
        5,
        "must subscribe hazard, turns, and the two FLCM status signals only"
    );
    let identities: Vec<(&str, &str)> = ids
        .iter()
        .map(|id| {
            (
                id.namespace.as_ref().unwrap().name.as_str(),
                id.name.as_str(),
            )
        })
        .collect();
    assert!(
        identities.contains(&(FLCM_NAMESPACE, LEFT_LOW_BEAM_STATUS_NAME))
            && identities.contains(&(FLCM_NAMESPACE, RIGHT_LOW_BEAM_STATUS_NAME)),
        "FLCM status identities must be subscribed: {identities:?}"
    );
    // Reject unlisted Hello World distractors; LowBeamLightStatus is allowed.
    let names: Vec<&str> = ids.iter().map(|id| id.name.as_str()).collect();
    for distractor in [
        "LowBeamLightControl",
        "HighBeam",
        "Brake",
        "TurnStalk",
        "Wiper",
        "Speed",
    ] {
        assert!(
            names.iter().all(|n| !n.contains(distractor)),
            "unexpected distractor {distractor} in {names:?}"
        );
    }
}

#[test]
fn subscription_ready_status_proves_connection_and_exact_target() {
    assert_eq!(
        subscription_ready_status(),
        "[remotive_bridge] connected; subscribed signals=\n\
SCCM-DriverCan0:HazardLightButton.HazardLightButton,\n\
BCM-BodyCan0:TurnLightControl.LeftTurnLightRequest,\n\
BCM-BodyCan0:TurnLightControl.RightTurnLightRequest,\n\
FLCM-BodyCan0:LowBeamLightStatus.LeftLowBeamLightStatus,\n\
FLCM-BodyCan0:LowBeamLightStatus.RightLowBeamLightStatus"
    );
}

#[test]
fn decoder_accepts_only_documented_boolean_encodings() {
    for (payload, expected) in [
        (Payload::Integer(0), false),
        (Payload::Integer(1), true),
        (Payload::Uinteger64(0), false),
        (Payload::Uinteger64(1), true),
        (Payload::StrValue("Off".into()), false),
        (Payload::StrValue("On".into()), true),
    ] {
        assert_eq!(decode_dbc_off_or_on(Some(&payload)), Some(expected));
    }

    let malformed = [
        None,
        Some(Payload::Empty(true)),
        Some(Payload::Double(1.0)),
        Some(Payload::Arbitration(true)),
        Some(Payload::Integer(-1)),
        Some(Payload::Integer(2)),
        Some(Payload::Uinteger64(2)),
        Some(Payload::StrValue("on".into())),
        Some(Payload::StrValue("invalid".into())),
    ];
    for payload in malformed {
        assert_eq!(decode_dbc_off_or_on(payload.as_ref()), None);
    }
}

#[test]
fn decoder_maps_dbc_ok_fail_integers_and_named_values() {
    for (payload, expected) in [
        (Payload::Integer(0), true),
        (Payload::Integer(1), false),
        (Payload::Uinteger64(0), true),
        (Payload::Uinteger64(1), false),
        (Payload::StrValue("Ok".into()), true),
        (Payload::StrValue("Fail".into()), false),
    ] {
        assert_eq!(decode_dbc_ok_or_fail(Some(&payload)), Some(expected));
    }

    let malformed = [
        None,
        Some(Payload::Empty(true)),
        Some(Payload::Double(1.0)),
        Some(Payload::Arbitration(true)),
        Some(Payload::Integer(-1)),
        Some(Payload::Integer(2)),
        Some(Payload::Uinteger64(2)),
        Some(Payload::StrValue("On".into())),
        Some(Payload::StrValue("Off".into())),
        Some(Payload::StrValue("ok".into())),
        Some(Payload::StrValue("fail".into())),
    ];
    for payload in malformed {
        assert_eq!(decode_dbc_ok_or_fail(payload.as_ref()), None);
    }
}

fn signal(namespace: Option<&str>, name: &str, payload: Option<Payload>) -> Signal {
    Signal {
        id: Some(SignalId {
            namespace: namespace.map(|name| NameSpace {
                name: name.to_owned(),
            }),
            name: name.to_owned(),
        }),
        raw: vec![],
        timestamp: 0,
        payload,
    }
}

#[test]
fn broker_batch_preserves_exact_hazard_right_left_signal_order() {
    let batch = Signals {
        signal: vec![
            signal(
                Some(HAZARD_NAMESPACE),
                HAZARD_NAME,
                Some(Payload::Integer(1)),
            ),
            signal(
                Some(TURN_NAMESPACE),
                RIGHT_TURN_NAME,
                Some(Payload::Integer(0)),
            ),
            signal(
                Some(TURN_NAMESPACE),
                LEFT_TURN_NAME,
                Some(Payload::StrValue("On".into())),
            ),
        ],
    };
    let mut rejected = ObservationRejectCounters::default();

    assert_eq!(
        decode_observation_signals(&batch, &mut rejected),
        vec![
            BrokerObservation::HazardButton(true),
            BrokerObservation::RightTurnRequest(false),
            BrokerObservation::LeftTurnRequest(true),
        ]
    );
    assert_eq!(rejected.wrong_identity(), 0);
    assert_eq!(rejected.invalid_payload(), 0);
}

#[test]
fn left_and_right_batches_are_valid_independently() {
    let left_only = Signals {
        signal: vec![signal(
            Some(TURN_NAMESPACE),
            LEFT_TURN_NAME,
            Some(Payload::Uinteger64(1)),
        )],
    };
    let right_only = Signals {
        signal: vec![signal(
            Some(TURN_NAMESPACE),
            RIGHT_TURN_NAME,
            Some(Payload::StrValue("Off".into())),
        )],
    };
    let mut rejected = ObservationRejectCounters::default();

    assert_eq!(
        decode_observation_signals(&left_only, &mut rejected),
        vec![BrokerObservation::LeftTurnRequest(true)]
    );
    assert_eq!(
        decode_observation_signals(&right_only, &mut rejected),
        vec![BrokerObservation::RightTurnRequest(false)]
    );
}

#[test]
fn values_from_separate_batches_remain_independent() {
    let mut rejected = ObservationRejectCounters::default();
    let left = Signals {
        signal: vec![signal(
            Some(TURN_NAMESPACE),
            LEFT_TURN_NAME,
            Some(Payload::Integer(1)),
        )],
    };
    let right = Signals {
        signal: vec![signal(
            Some(TURN_NAMESPACE),
            RIGHT_TURN_NAME,
            Some(Payload::Integer(1)),
        )],
    };

    assert_eq!(
        decode_observation_signals(&left, &mut rejected),
        vec![BrokerObservation::LeftTurnRequest(true)]
    );
    assert_eq!(
        decode_observation_signals(&right, &mut rejected),
        vec![BrokerObservation::RightTurnRequest(true)]
    );
}

#[test]
fn flcm_status_maps_dbc_zero_to_ok_true_and_one_to_fail_false() {
    let batch = Signals {
        signal: vec![
            signal(
                Some(FLCM_NAMESPACE),
                LEFT_LOW_BEAM_STATUS_NAME,
                Some(Payload::Integer(0)),
            ),
            signal(
                Some(FLCM_NAMESPACE),
                RIGHT_LOW_BEAM_STATUS_NAME,
                Some(Payload::Integer(1)),
            ),
        ],
    };
    let mut rejected = ObservationRejectCounters::default();

    assert_eq!(
        decode_observation_signals(&batch, &mut rejected),
        vec![
            BrokerObservation::LeftLowBeamStatus(true),
            BrokerObservation::RightLowBeamStatus(false),
        ]
    );
    assert_eq!(rejected.wrong_identity(), 0);
    assert_eq!(rejected.invalid_payload(), 0);
}

#[test]
fn flcm_status_left_and_right_batches_are_valid_independently() {
    let left_ok = Signals {
        signal: vec![signal(
            Some(FLCM_NAMESPACE),
            LEFT_LOW_BEAM_STATUS_NAME,
            Some(Payload::Uinteger64(0)),
        )],
    };
    let right_fail = Signals {
        signal: vec![signal(
            Some(FLCM_NAMESPACE),
            RIGHT_LOW_BEAM_STATUS_NAME,
            Some(Payload::StrValue("Fail".into())),
        )],
    };
    let mut rejected = ObservationRejectCounters::default();

    assert_eq!(
        decode_observation_signals(&left_ok, &mut rejected),
        vec![BrokerObservation::LeftLowBeamStatus(true)]
    );
    assert_eq!(
        decode_observation_signals(&right_fail, &mut rejected),
        vec![BrokerObservation::RightLowBeamStatus(false)]
    );
    assert_eq!(rejected.wrong_identity(), 0);
    assert_eq!(rejected.invalid_payload(), 0);
}

#[test]
fn decoder_rejects_wrong_identity_and_malformed_payload() {
    let batch = Signals {
        signal: vec![
            signal(
                Some("wrong-namespace"),
                HAZARD_NAME,
                Some(Payload::Integer(0)),
            ),
            signal(
                Some(TURN_NAMESPACE),
                "wrong-name",
                Some(Payload::Integer(0)),
            ),
            signal(None, LEFT_TURN_NAME, Some(Payload::Integer(0))),
            Signal {
                id: None,
                raw: vec![],
                timestamp: 0,
                payload: Some(Payload::Integer(0)),
            },
            signal(
                Some(TURN_NAMESPACE),
                RIGHT_TURN_NAME,
                Some(Payload::Integer(2)),
            ),
        ],
    };
    let mut rejected = ObservationRejectCounters::default();

    assert!(decode_observation_signals(&batch, &mut rejected).is_empty());
    assert_eq!(rejected.wrong_identity(), 4);
    assert_eq!(rejected.invalid_payload(), 1);
}

#[test]
fn cli_defaults_and_overrides_are_emulator_compatible() {
    let defaults = parse_args(std::iter::empty::<&str>()).unwrap();
    assert_eq!(defaults.broker_url, DEFAULT_BROKER_URL);
    assert_eq!(defaults.can_interface, DEFAULT_CAN_INTERFACE);
    assert_eq!(
        DEFAULT_CAN_INTERFACE,
        vehicle_device_bus::DEFAULT_CAN_INTERFACE
    );
    assert_eq!(defaults.tick, Duration::from_millis(DEFAULT_TICK_MS));
    assert_eq!(defaults.readings, None);
    assert_eq!(defaults.rpm_clamp, DEFAULT_RPM_CLAMP);
    assert!(
        defaults.rpm_clamp > common::RPM_DRIVING_THRESHOLD,
        "default clamp must allow Driving (rpm > {})",
        common::RPM_DRIVING_THRESHOLD
    );

    let args = parse_args([
        "--broker-url",
        "http://broker:50051",
        "--can-interface",
        "can7",
        "--tick-ms",
        "250",
        "--readings",
        "3",
        "--rpm-clamp",
        "3000",
    ])
    .unwrap();
    assert_eq!(args.broker_url, "http://broker:50051");
    assert_eq!(args.can_interface, "can7");
    assert_eq!(args.tick, Duration::from_millis(250));
    assert_eq!(args.readings, NonZeroUsize::new(3));
    assert_eq!(args.rpm_clamp, 3000);
}

#[test]
fn cli_rejects_empty_or_non_positive_values_and_unknown_flags() {
    assert!(parse_args(["--broker-url", ""]).is_err());
    assert!(parse_args(["--can-interface", ""]).is_err());
    assert!(parse_args(["--tick-ms", "0"]).is_err());
    assert!(parse_args(["--readings", "0"]).is_err());
    assert!(parse_args(["--rpm-clamp", "not-a-number"]).is_err());
    assert!(parse_args(["--unknown"]).is_err());
}

#[tokio::test]
async fn observation_rpm_stays_at_or_below_configured_clamp() {
    let mut source = ProfileRpmSource::new(Duration::from_millis(1), DEFAULT_RPM_CLAMP);
    for _ in 0..200 {
        let rpm = source.next_rpm().await.unwrap();
        assert!(
            rpm <= DEFAULT_RPM_CLAMP,
            "default clamp must keep rpm ≤ {DEFAULT_RPM_CLAMP}, got {rpm}"
        );
        assert!(rpm > common::RPM_DRIVING_THRESHOLD);
        assert!(
            common::calculate_speed_from_rpm(rpm)
                <= f64::from(common::SPEED_EXTREME_OPERATION_THRESHOLD_KPH),
            "cruise rpm {rpm} must stay at or under the 160 km/h warning line"
        );
    }
}

#[tokio::test]
async fn observation_rpm_respects_raised_clamp() {
    let clamp = 2500u16;
    let mut source = ProfileRpmSource::new(Duration::from_millis(1), clamp);
    let mut saw_above_driving = false;
    for _ in 0..400 {
        let rpm = source.next_rpm().await.unwrap();
        assert!(rpm <= clamp, "rpm {rpm} exceeded clamp {clamp}");
        if rpm > common::RPM_DRIVING_THRESHOLD {
            saw_above_driving = true;
        }
    }
    assert!(
        saw_above_driving,
        "raised clamp should allow profile above Idle driving threshold"
    );
}
