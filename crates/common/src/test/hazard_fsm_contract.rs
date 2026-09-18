use std::collections::BTreeSet;
use std::time::Instant;

use crate::fsm::{AssemblyId, DomainAction, FsmEvent, FsmState, transition};
use crate::twin_runtime::controller::AssemblyTopology;
use crate::twin_runtime::{ZoneReplies, run_to_quiescence, twin_turn};
use crate::vehicle_state::{BcmState, ObservedBool, VehicleContext};

#[test]
fn power_lifecycle_waits_for_sccm_and_bcm() {
    let now = Instant::now();
    let ctx = VehicleContext::default();
    let expected = BTreeSet::from([AssemblyId::Sccm, AssemblyId::Bcm]);

    let starting = transition(&FsmState::Off, &FsmEvent::PowerOn, &ctx, now);
    assert_eq!(
        starting.next_state,
        FsmState::PreparingToStart(expected.clone())
    );
    let after_sccm = transition(
        &starting.next_state,
        &FsmEvent::AssemblyZoneReady(AssemblyId::Sccm),
        &ctx,
        now,
    );
    assert_eq!(
        after_sccm.next_state,
        FsmState::PreparingToStart(BTreeSet::from([AssemblyId::Bcm]))
    );
    let ready = transition(
        &after_sccm.next_state,
        &FsmEvent::AssemblyZoneReady(AssemblyId::Bcm),
        &ctx,
        now,
    );
    assert_eq!(ready.next_state, FsmState::Idle);

    let stopping = transition(&FsmState::Idle, &FsmEvent::PowerOff, &ctx, now);
    assert_eq!(stopping.next_state, FsmState::PreparingToStop(expected));
}

#[test]
fn hazard_self_loops_in_every_active_mode_before_unrelated_context_guards() {
    let now = Instant::now();

    let mut driving_ctx = VehicleContext::default();
    driving_ctx.powertrain.apply_rpm(7_500);
    driving_ctx.powertrain.refresh_speed();

    let mut dangerous_ctx = VehicleContext::default();
    dangerous_ctx.headlamp.state = crate::vehicle_state::HeadlampState::On;
    dangerous_ctx.visibility.ambient_lux = u16::MAX;

    let warning_ctx = VehicleContext::default();

    for (state, ctx) in [
        (FsmState::Idle, VehicleContext::default()),
        (FsmState::Driving, driving_ctx),
        (FsmState::DrivingDangerously, dangerous_ctx),
        (FsmState::ExtremeOperationWarning(now), warning_ctx),
    ] {
        let result = transition(&state, &FsmEvent::HazardButtonObserved(true), &ctx, now);
        assert_eq!(result.next_state, state, "hazard moved active mode");
        assert!(result.note.is_none());
    }
}

#[test]
fn observed_hazard_updates_sccm_without_bcm_turn_computation() {
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

        let result = twin_turn(&state, &ctx, &FsmEvent::HazardButtonObserved(true), now);
        assert_eq!(result.next_state, state);
        assert_eq!(result.modified_ctx.sccm.hazard_button, ObservedBool::On);
        assert_eq!(
            result.modified_ctx.bcm.left_turn_request,
            ObservedBool::Unknown
        );
        assert_eq!(
            result.modified_ctx.bcm.right_turn_request,
            ObservedBool::Unknown
        );
        assert!(
            !result
                .actions
                .iter()
                .any(|action| matches!(action, DomainAction::SetTurnLights { .. }))
        );
    }
}

#[test]
fn duplicate_active_hazard_changed_still_records_self_loop_without_action() {
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
        AssemblyTopology::ObservedEcus,
    );
    assert_eq!(result.hops.len(), 1);
    assert_eq!(result.final_step().next_state, FsmState::Driving);
}
