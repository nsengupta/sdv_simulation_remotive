//! Unit tests for tell-back retry / synthetic embed policy.

use crate::fsm::DomainAction;
use crate::fsm::{FsmEvent, FsmState};
use crate::twin_runtime::constants::ZONE_TELL_BACK_MAX_RETRIES;
use crate::twin_runtime::zone_tell_back::{
    TellBackTimeoutOutcome, TellBackWait, on_tell_back_timeout,
    synthetic_unresponsive_headlamp_reply,
};
use crate::twin_runtime::{ResolvedTurn, ZoneReplies, commit_resolved_turn};
use crate::vehicle_state::{HeadlampContext, HeadlampOutcome};
use std::time::Instant;

#[test]
fn tell_back_wait_retries_then_synthesizes_unresponsive_embed() {
    let ctx = HeadlampContext::default();
    let mut wait = TellBackWait::new(7);

    for remaining in (0..ZONE_TELL_BACK_MAX_RETRIES).rev() {
        assert_eq!(wait.retries_remaining, remaining + 1);
        match on_tell_back_timeout(&ctx, wait) {
            TellBackTimeoutOutcome::Retry(next) => wait = next,
            TellBackTimeoutOutcome::Exhausted(_) => panic!("expected retry"),
        }
    }

    match on_tell_back_timeout(&ctx, wait) {
        TellBackTimeoutOutcome::Exhausted(reply) => {
            assert_eq!(reply.ctx, ctx);
            assert!(matches!(
                &reply.outcomes[0],
                HeadlampOutcome::LogWarning(msg) if msg.contains("unresponsive")
            ));
        }
        TellBackTimeoutOutcome::Retry(_) => panic!("expected synthetic embed"),
    }
}

#[test]
fn synthetic_unresponsive_embed_surfaces_log_warning_on_commit() {
    let t0 = Instant::now();
    let ctx = HeadlampContext::default();
    let synthetic = synthetic_unresponsive_headlamp_reply(&ctx);
    let quiescent = commit_resolved_turn(
        &FsmState::Driving,
        &driving_ctx(),
        ResolvedTurn {
            ingress: FsmEvent::TimerTick,
            now: t0,
            zone_replies: ZoneReplies::with_reply(
                crate::fsm::AssemblyId::Headlamp,
                crate::digital_twin::ZoneReply::Headlamp(synthetic),
            ),
        },
    );
    assert!(
        quiescent.merged_actions().iter().any(|a| {
            matches!(a, DomainAction::LogWarning(msg) if msg.contains("unresponsive"))
        }),
        "ledger path must carry unresponsive warning, got {:?}",
        quiescent.merged_actions()
    );
}

fn driving_ctx() -> crate::vehicle_state::VehicleContext {
    use crate::vehicle_physics::{LUX_ON_THRESHOLD, RPM_DRIVING_THRESHOLD};
    let mut ctx = crate::vehicle_state::VehicleContext::default();
    ctx.visibility.ambient_lux = LUX_ON_THRESHOLD;
    ctx.powertrain.apply_rpm(RPM_DRIVING_THRESHOLD + 100);
    ctx.powertrain.refresh_speed();
    ctx
}

#[tokio::test]
async fn given_silent_headlamp_when_headlamp_demux_event_then_ledger_records_unresponsive_warning()
{
    use crate::fsm::{FsmEvent, FsmState};
    use crate::test::ActorGuard;
    use crate::twin_runtime::constants::{ZONE_TELL_BACK_ATTEMPT_COUNT, ZONE_TELL_BACK_WAIT};
    use crate::twin_runtime::controller::vehicle_controller::{
        AssemblyTopology, VehicleControllerRuntimeOptions,
    };
    use crate::{PublishedDomainAction, PublishedFsmEvent, VehicleController};
    use tokio::sync::mpsc;

    let (transition_tx, mut rx) = mpsc::channel(16);
    let runtime_options = VehicleControllerRuntimeOptions {
        assembly_topology: AssemblyTopology::Legacy,
        transition_tx: Some(transition_tx),
        test_silent_headlamp: true,
        ..VehicleControllerRuntimeOptions::default()
    };

    let (controller, handle) = VehicleController::install_and_start_with_options(
        "ZONE-TELL-BACK-01".to_string(),
        runtime_options,
    )
    .await
    .expect("start actor");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    // Headlamp is not a Phase I lifecycle participant; BCM starts automatically.
    controller.send_power_on().await.expect("power on");
    crate::test::wait_fsm_state(
        &controller,
        FsmState::Idle,
        std::time::Duration::from_millis(500),
    )
    .await;
    // Drain PowerOn + SCCM ready + BCM ready.
    let _ = rx.recv().await.expect("power on row");
    let _ = rx.recv().await.expect("SCCM assembly zone ready row");
    let _ = rx.recv().await.expect("BCM assembly zone ready row");

    // Now the headlamp is still silent for operational tell-backs.
    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(500))
        .await
        .expect("lux ingress that tells headlamp");

    let wait_budget = ZONE_TELL_BACK_WAIT
        .saturating_mul(ZONE_TELL_BACK_ATTEMPT_COUNT as u32)
        .saturating_add(std::time::Duration::from_millis(100));
    let record = tokio::time::timeout(wait_budget, rx.recv())
        .await
        .expect("ledger row within tell-back retry budget")
        .expect("lux row");

    assert_eq!(record.event, PublishedFsmEvent::UpdateAmbientLux(500));
    assert!(
        record
            .actions
            .iter()
            .any(|action| matches!(action, PublishedDomainAction::LogWarning(msg) if msg.contains("unresponsive"))),
        "transition log must record headlamp unresponsive, got {:?}",
        record.actions
    );
}
