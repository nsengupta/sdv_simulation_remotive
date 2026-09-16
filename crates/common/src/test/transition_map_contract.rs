//! Contract tests for [`crate::fsm::transition_map`] (operational mode table).

use crate::fsm::{AssemblyId, FsmAction, FsmEvent, FsmState, output, transition};
use crate::vehicle_physics::{EXTREME_OPERATION_WARNING_MESSAGE, SPEED_THRESHOLD_WARNING_MESSAGE};
use crate::vehicle_state::VehicleContext;
use std::time::Instant;

fn valid_twin_context() -> VehicleContext {
    VehicleContext::default()
}

#[test]
fn given_driving_when_high_rpm_then_enters_extreme_operation_warning() {
    let now = Instant::now();
    let mut ctx = valid_twin_context();
    ctx.powertrain.wheel_rpm.front_left = 5600;
    ctx.powertrain.refresh_speed();

    let result = transition(&FsmState::Driving, &FsmEvent::UpdateRpm(5600), &ctx, now);

    assert!(matches!(
        result.next_state,
        FsmState::ExtremeOperationWarning(_)
    ));
}

#[test]
fn given_driving_when_both_extreme_thresholds_exceeded_then_emits_both_warnings() {
    let now = Instant::now();
    let mut ctx = valid_twin_context();
    ctx.powertrain.wheel_rpm.front_left = 5600;
    ctx.powertrain.refresh_speed();

    let result = transition(&FsmState::Driving, &FsmEvent::UpdateRpm(5600), &ctx, now);
    assert!(matches!(
        result.next_state,
        FsmState::ExtremeOperationWarning(_)
    ));

    let actions = output(&FsmState::Driving, &result.next_state, &ctx);
    assert!(actions.contains(&FsmAction::LogWarning(
        SPEED_THRESHOLD_WARNING_MESSAGE.to_string()
    )));
    assert!(actions.contains(&FsmAction::LogWarning(
        EXTREME_OPERATION_WARNING_MESSAGE.to_string()
    )));
}

#[test]
fn given_warning_recovery_when_transition_to_driving_then_stops_buzzer() {
    let old_state = FsmState::ExtremeOperationWarning(Instant::now());
    let new_state = FsmState::Driving;
    let ctx = valid_twin_context();

    let actions = output(&old_state, &new_state, &ctx);
    assert_eq!(actions, vec![FsmAction::StopBuzzer]);
}

/// FLCM low-beam status arrives at ~20 Hz and its silence verdict arrives out of band as
/// `AssemblyZoneReady(Flcm)`. Neither may nudge the operational mode, so both must hit the
/// explicit self-loop arms rather than the context-derived catch-alls.
#[test]
fn given_flcm_observation_or_silence_then_operational_mode_self_loops() {
    let now = Instant::now();
    let mut speeding = valid_twin_context();
    speeding.powertrain.wheel_rpm.front_left = 5600;
    speeding.powertrain.refresh_speed();
    let mut stationary = valid_twin_context();
    stationary.powertrain.apply_rpm(0);
    stationary.powertrain.refresh_speed();

    let flcm_events = [
        FsmEvent::LeftLowBeamStatusObserved(true),
        FsmEvent::LeftLowBeamStatusObserved(false),
        FsmEvent::RightLowBeamStatusObserved(true),
        FsmEvent::RightLowBeamStatusObserved(false),
        FsmEvent::AssemblyZoneReady(AssemblyId::Flcm),
    ];
    let states = [
        FsmState::Idle,
        FsmState::Driving,
        FsmState::DrivingDangerously,
        FsmState::ExtremeOperationWarning(now),
    ];

    for state in &states {
        for event in &flcm_events {
            // A context that would otherwise force a mode change through the catch-all arms.
            for ctx in [&speeding, &stationary] {
                let result = transition(state, event, ctx, now);
                assert_eq!(
                    &result.next_state, state,
                    "{state:?} + {event:?} must self-loop"
                );
                assert_eq!(result.note, None, "{state:?} + {event:?} must be routine");
            }
        }
    }
}

#[test]
fn given_extreme_warning_when_headlamp_ack_and_stationary_then_idle() {
    let now = Instant::now();
    let warning = FsmState::ExtremeOperationWarning(now);
    let mut ctx = valid_twin_context();
    ctx.powertrain.apply_rpm(0);
    ctx.powertrain.refresh_speed();

    let result = transition(&warning, &FsmEvent::FrontHeadlampOffAck, &ctx, now);
    assert_eq!(
        result.next_state,
        FsmState::Idle,
        "standstill must exit ExtremeOperationWarning on any non-PowerOff event"
    );
}
