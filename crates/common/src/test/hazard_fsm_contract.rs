use std::collections::BTreeSet;
use std::time::Instant;

use crate::fsm::{AssemblyId, DomainAction, FsmEvent, FsmState, transition};
use crate::twin_runtime::{ZoneReplies, run_to_quiescence, twin_turn};
use crate::vehicle_state::{BcmState, VehicleContext};

#[test]
fn power_lifecycle_waits_for_bcm_only() {
    let now = Instant::now();
    let ctx = VehicleContext::default();
    let expected = BTreeSet::from([AssemblyId::Bcm]);

    let starting = transition(&FsmState::Off, &FsmEvent::PowerOn, &ctx, now);
    assert_eq!(
        starting.next_state,
        FsmState::PreparingToStart(expected.clone())
    );
    let ready = transition(
        &starting.next_state,
        &FsmEvent::AssemblyZoneReady(AssemblyId::Bcm),
        &ctx,
        now,
    );
    assert_eq!(ready.next_state, FsmState::Idle);

    let stopping = transition(&FsmState::Idle, &FsmEvent::PowerOff, &ctx, now);
    assert_eq!(stopping.next_state, FsmState::PreparingToStop(expected));
}

#[test]
fn active_hazard_updates_sccm_and_bcm_and_maps_one_atomic_action() {
    let now = Instant::now();
    for state in [
        FsmState::Idle,
        FsmState::Driving,
        FsmState::DrivingDangerously,
        FsmState::ExtremeOperationWarning(now),
    ] {
        let mut ctx = VehicleContext::default();
        ctx.bcm.state = BcmState::Ready;
        ctx.powertrain.apply_rpm(500);
        ctx.powertrain.refresh_speed();

        let result = twin_turn(&state, &ctx, &FsmEvent::HazardButtonChanged(true), now);
        assert_eq!(result.next_state, state);
        assert!(result.modified_ctx.sccm.hazard_button_on);
        assert!(result.modified_ctx.bcm.left_turn_request_on);
        assert!(result.modified_ctx.bcm.right_turn_request_on);
        assert_eq!(
            result.actions,
            vec![DomainAction::SetTurnLights {
                left_on: true,
                right_on: true,
            }]
        );
    }
}

#[test]
fn duplicate_active_hazard_still_records_self_loop_without_action() {
    let now = Instant::now();
    let mut ctx = VehicleContext::default();
    ctx.sccm.hazard_button_on = true;
    ctx.bcm.state = BcmState::Ready;
    ctx.bcm.left_turn_request_on = true;
    ctx.bcm.right_turn_request_on = true;

    let result = twin_turn(
        &FsmState::Idle,
        &ctx,
        &FsmEvent::HazardButtonChanged(true),
        now,
    );
    assert_eq!(result.next_state, FsmState::Idle);
    assert!(result.actions.is_empty());
    assert_eq!(
        result.transition_record.event,
        FsmEvent::HazardButtonChanged(true)
    );
}

#[test]
fn phase_one_rpm_entry_stays_driving_with_inactive_headlamp() {
    let result = run_to_quiescence(
        &FsmState::Idle,
        &VehicleContext::default(),
        &FsmEvent::UpdateRpm(1_500),
        Instant::now(),
        &ZoneReplies::simulate_locally(),
    );
    assert_eq!(result.hops.len(), 1);
    assert_eq!(result.final_step().next_state, FsmState::Driving);
}
