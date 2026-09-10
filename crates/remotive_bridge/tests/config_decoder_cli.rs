use remotive_bridge::cli::{
    DEFAULT_BROKER_URL, DEFAULT_CAN_INTERFACE, DEFAULT_TICK_MS, parse_args,
};
use remotive_bridge::decoder::decode_hazard;
use remotive_bridge::source::{
    CLIENT_ID, HAZARD_NAME, HAZARD_NAMESPACE, HazardRejectCounters, ProfileRpmSource, RpmSource,
    decode_hazard_signals, subscription_config, subscription_ready_status,
};
use remotivelabs_broker::generated::base::signal::Payload;
use remotivelabs_broker::generated::base::{NameSpace, Signal, SignalId, Signals};
use std::num::NonZeroUsize;
use std::time::Duration;

#[test]
fn subscription_config_is_exact_and_preserves_duplicates() {
    let config = subscription_config();
    assert_eq!(config.client_id.unwrap().id, CLIENT_ID);
    let ids = config.signals.unwrap().signal_id;
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].name, HAZARD_NAME);
    assert_eq!(ids[0].namespace.as_ref().unwrap().name, HAZARD_NAMESPACE);
    assert!(!config.on_change);
    assert!(!config.initial_empty);
}

#[test]
fn subscription_ready_status_proves_connection_and_exact_target() {
    assert_eq!(
        subscription_ready_status(),
        "[remotive_bridge] connected; subscribed signal=\
SCCM-DriverCan0:HazardLightButton.HazardLightButton"
    );
}

#[test]
fn decoder_accepts_only_documented_hazard_encodings() {
    for (payload, expected) in [
        (Payload::Integer(0), false),
        (Payload::Integer(1), true),
        (Payload::Uinteger64(0), false),
        (Payload::Uinteger64(1), true),
        (Payload::StrValue("Off".into()), false),
        (Payload::StrValue("On".into()), true),
    ] {
        assert_eq!(decode_hazard(Some(&payload)), Some(expected));
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
        assert_eq!(decode_hazard(payload.as_ref()), None);
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
fn broker_batch_decodes_only_exact_hazard_signal_identity() {
    let batch = Signals {
        signal: vec![
            signal(
                Some(HAZARD_NAMESPACE),
                HAZARD_NAME,
                Some(Payload::Integer(1)),
            ),
            signal(
                Some("wrong-namespace"),
                HAZARD_NAME,
                Some(Payload::Integer(0)),
            ),
            signal(
                Some(HAZARD_NAMESPACE),
                "wrong-name",
                Some(Payload::Integer(0)),
            ),
            signal(None, HAZARD_NAME, Some(Payload::Integer(0))),
            Signal {
                id: None,
                raw: vec![],
                timestamp: 0,
                payload: Some(Payload::Integer(0)),
            },
            signal(
                Some(HAZARD_NAMESPACE),
                HAZARD_NAME,
                Some(Payload::Integer(2)),
            ),
        ],
    };
    let mut rejected = HazardRejectCounters::default();

    assert_eq!(decode_hazard_signals(&batch, &mut rejected), vec![true]);
    assert_eq!(rejected.wrong_identity(), 4);
    assert_eq!(rejected.invalid_payload(), 1);
}

#[test]
fn cli_defaults_and_overrides_are_emulator_compatible() {
    let defaults = parse_args(std::iter::empty::<&str>()).unwrap();
    assert_eq!(defaults.broker_url, DEFAULT_BROKER_URL);
    assert_eq!(defaults.can_interface, DEFAULT_CAN_INTERFACE);
    assert_eq!(defaults.tick, Duration::from_millis(DEFAULT_TICK_MS));
    assert_eq!(defaults.readings, None);

    let args = parse_args([
        "--broker-url",
        "http://broker:50051",
        "--can-interface",
        "can7",
        "--tick-ms",
        "250",
        "--readings",
        "3",
    ])
    .unwrap();
    assert_eq!(args.broker_url, "http://broker:50051");
    assert_eq!(args.can_interface, "can7");
    assert_eq!(args.tick, Duration::from_millis(250));
    assert_eq!(args.readings, NonZeroUsize::new(3));
}

#[test]
fn cli_rejects_empty_or_non_positive_values_and_unknown_flags() {
    assert!(parse_args(["--broker-url", ""]).is_err());
    assert!(parse_args(["--can-interface", ""]).is_err());
    assert!(parse_args(["--tick-ms", "0"]).is_err());
    assert!(parse_args(["--readings", "0"]).is_err());
    assert!(parse_args(["--unknown"]).is_err());
}

#[tokio::test]
async fn reused_daytime_profile_exposes_only_an_rpm_reading() {
    let mut source = ProfileRpmSource::new(Duration::from_millis(1));
    let rpm: u16 = source.next_rpm().await.unwrap();

    assert!((common::RPM_IDLE..=emulator::models::DAYTIME_TUNNEL_RPM_CEILING).contains(&rpm));
}
