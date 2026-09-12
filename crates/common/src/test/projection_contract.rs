//! Contract tests for projection from canonical twin ingress into the FSM mailbox.

use crate::digital_twin::TwinMessage;
use crate::fsm::FsmEvent;
use crate::twin_runtime::connectors::{IngressToFsmProjector, ProjectionError, Projector};
use crate::{ControlSignal, LifecycleCommand, ObservedEcuSignal, TwinIngressEvent, VssSignal};

#[test]
fn canonical_twin_ingress_names_are_public_and_projectable() {
    use crate::{IngressToFsmProjector, LifecycleCommand, TwinIngressEvent, TwinMessage};

    let projector = IngressToFsmProjector;
    let output = projector
        .project(TwinIngressEvent::Telemetry(VssSignal::Speed(50.0)))
        .expect_err("observed speed remains unsupported by the FSM");

    assert!(matches!(output, ProjectionError::InvalidPayload(_)));
    assert_eq!(LifecycleCommand::PowerOn, LifecycleCommand::PowerOn);
    let _: Option<TwinMessage> = None;
}

#[test]
fn given_timer_tick_when_projected_then_maps_to_fsm_timer_tick() {
    let projector = IngressToFsmProjector;
    let out = projector
        .project(TwinIngressEvent::TimerTick)
        .expect("projection must succeed");
    match out {
        TwinMessage::Fsm(FsmEvent::TimerTick) => {}
        other => panic!("unexpected timer tick mapping: {other:?}"),
    }
}

#[test]
fn given_system_reset_when_projected_then_maps_to_fsm_power_off() {
    let projector = IngressToFsmProjector;
    let out = projector
        .project(TwinIngressEvent::SystemReset)
        .expect("projection must succeed");
    match out {
        TwinMessage::Fsm(FsmEvent::PowerOff) => {}
        other => panic!("unexpected reset mapping: {other:?}"),
    }
}

#[test]
fn lifecycle_power_on_projects_to_fsm_power_on() {
    let out = IngressToFsmProjector
        .project(TwinIngressEvent::Lifecycle(LifecycleCommand::PowerOn))
        .expect("PowerOn projection");

    assert!(matches!(out, TwinMessage::Fsm(FsmEvent::PowerOn)));
}

#[test]
fn lifecycle_power_off_projects_to_fsm_power_off() {
    let out = IngressToFsmProjector
        .project(TwinIngressEvent::Lifecycle(LifecycleCommand::PowerOff))
        .expect("PowerOff projection");

    assert!(matches!(out, TwinMessage::Fsm(FsmEvent::PowerOff)));
}

#[test]
fn given_observed_speed_signal_when_projected_then_rejects_until_ecu_path_exists() {
    let projector = IngressToFsmProjector;
    let err = projector
        .project(TwinIngressEvent::Telemetry(VssSignal::Speed(50.0)))
        .expect_err("observed speed must not ingress as UpdateSpeed");
    assert!(matches!(err, ProjectionError::InvalidPayload(_)));
}

#[test]
fn given_rpm_signal_when_projected_then_maps_exact_rpm() {
    let projector = IngressToFsmProjector;
    let out = projector
        .project(TwinIngressEvent::Telemetry(VssSignal::EngineRpm(4321)))
        .expect("rpm projection must succeed");
    match out {
        TwinMessage::Fsm(FsmEvent::UpdateRpm(v)) => assert_eq!(v, 4321),
        other => panic!("unexpected rpm mapping: {other:?}"),
    }
}

#[test]
fn given_hazard_button_when_projected_then_maps_exact_state() {
    let projector = IngressToFsmProjector;
    for pressed in [false, true] {
        let out = projector
            .project(TwinIngressEvent::Control(ControlSignal::HazardButton(
                pressed,
            )))
            .expect("hazard projection must succeed");
        assert!(matches!(
            out,
            TwinMessage::Fsm(FsmEvent::HazardButtonChanged(actual)) if actual == pressed
        ));
    }
}

#[test]
fn observed_hazard_projects_to_hazard_button_observed_not_changed() {
    let out = IngressToFsmProjector
        .project(TwinIngressEvent::ObservedEcu(
            ObservedEcuSignal::HazardButton(true),
        ))
        .expect("observed hazard projection");

    assert!(matches!(
        out,
        TwinMessage::Fsm(FsmEvent::HazardButtonObserved(true))
    ));
    assert!(!matches!(
        out,
        TwinMessage::Fsm(FsmEvent::HazardButtonChanged(_))
    ));
}

#[test]
fn observed_left_turn_projects_to_left_turn_request_observed() {
    let out = IngressToFsmProjector
        .project(TwinIngressEvent::ObservedEcu(
            ObservedEcuSignal::LeftTurnRequest(true),
        ))
        .expect("observed left projection");

    assert!(matches!(
        out,
        TwinMessage::Fsm(FsmEvent::LeftTurnRequestObserved(true))
    ));
}

#[test]
fn observed_right_turn_projects_to_right_turn_request_observed() {
    let out = IngressToFsmProjector
        .project(TwinIngressEvent::ObservedEcu(
            ObservedEcuSignal::RightTurnRequest(false),
        ))
        .expect("observed right projection");

    assert!(matches!(
        out,
        TwinMessage::Fsm(FsmEvent::RightTurnRequestObserved(false))
    ));
}

#[test]
fn unsupported_phase_one_telemetry_is_rejected() {
    let projector = IngressToFsmProjector;
    for signal in [
        VssSignal::Speed(50.0),
        VssSignal::AmbientLux(28),
        VssSignal::RainDetected(true),
    ] {
        let err = projector
            .project(TwinIngressEvent::Telemetry(signal))
            .expect_err("unsupported Phase I telemetry must be rejected");
        assert!(matches!(err, ProjectionError::InvalidPayload(_)));
    }
}

#[test]
fn given_front_headlamp_on_confirmed_when_projected_then_maps_to_fsm() {
    let projector = IngressToFsmProjector;
    let out = projector
        .project(TwinIngressEvent::FrontHeadlampCommandConfirmed { on_command: true })
        .expect("on ack projection must succeed");
    match out {
        TwinMessage::Fsm(FsmEvent::FrontHeadlampOnAck) => {}
        other => panic!("unexpected on ack mapping: {other:?}"),
    }
}

#[test]
fn given_front_headlamp_off_confirmed_when_projected_then_maps_to_fsm() {
    let projector = IngressToFsmProjector;
    let out = projector
        .project(TwinIngressEvent::FrontHeadlampCommandConfirmed { on_command: false })
        .expect("off ack projection must succeed");
    match out {
        TwinMessage::Fsm(FsmEvent::FrontHeadlampOffAck) => {}
        other => panic!("unexpected off ack mapping: {other:?}"),
    }
}

// ── Headlamp rejected ─────────────────────────────────────────────────────────

#[test]
fn given_front_headlamp_rejected_when_projected_then_maps_to_incomplete_with_negative_ack() {
    let projector = IngressToFsmProjector;
    let out = projector
        .project(TwinIngressEvent::FrontHeadlampCommandRejected { on_command: true })
        .expect("reject projection must succeed");
    match out {
        TwinMessage::Fsm(FsmEvent::FrontHeadlampActuationIncomplete { direction, cause }) => {
            assert!(matches!(
                direction,
                crate::fsm::FrontHeadlampSwitchDirection::On
            ));
            assert!(matches!(
                cause,
                crate::fsm::FrontHeadlampIncompleteCause::NegativeAck
            ));
        }
        other => panic!("unexpected rejected mapping: {other:?}"),
    }
}
