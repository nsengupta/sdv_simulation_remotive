use std::time::Duration;

use async_trait::async_trait;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use tokio::sync::mpsc;

use crate::digital_twin::{TwinMessage, ZoneReply};
use crate::fsm::{AssemblyId, DomainAction, FsmEvent, FsmState};
use crate::test::{ActorGuard, power_on_to_idle, wait_fsm_state};
use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;
use crate::twin_runtime::{BcmActor, BcmActorMsg, BcmActorState, tell_bcm_zone};
use crate::vehicle_state::{
    BcmContext, BcmMessage, BcmOutcome, BcmState, BcmZoneReply, HeadlampContext, HeadlampZoneReply,
    ObservationDisposition, ObservedBool,
};
use crate::{PublishedDomainAction, PublishedFsmEvent, VehicleController};

#[derive(Default)]
struct ReadyCollector;

struct CollectorState {
    tx: mpsc::UnboundedSender<TwinMessage>,
}

#[async_trait]
impl Actor for ReadyCollector {
    type Msg = TwinMessage;
    type State = CollectorState;
    type Arguments = mpsc::UnboundedSender<TwinMessage>;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        tx: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(CollectorState { tx })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        let _ = state.tx.send(message);
        Ok(())
    }
}

async fn spawn_bcm_child(
    silent: bool,
) -> (
    ActorRef<BcmActorMsg>,
    ActorRef<TwinMessage>,
    mpsc::UnboundedReceiver<TwinMessage>,
    ActorGuard<BcmActorMsg>,
    ActorGuard<TwinMessage>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let (brain, brain_handle) = ractor::spawn::<ReadyCollector>(tx)
        .await
        .expect("collector");
    let (bcm, bcm_handle) = ractor::spawn::<BcmActor>(BcmActorState::new(
        BcmContext {
            state: BcmState::Ready,
            ..Default::default()
        },
        silent,
        brain.clone(),
    ))
    .await
    .expect("bcm actor");
    (
        bcm.clone(),
        brain.clone(),
        rx,
        ActorGuard {
            addr: bcm,
            handle: bcm_handle,
        },
        ActorGuard {
            addr: brain,
            handle: brain_handle,
        },
    )
}

async fn next_ready(rx: &mut mpsc::UnboundedReceiver<TwinMessage>) -> TwinMessage {
    tokio::time::timeout(Duration::from_millis(250), rx.recv())
        .await
        .expect("bcm tell-back timeout")
        .expect("collector closed")
}

fn observed_reply(left: ObservedBool, right: ObservedBool) -> ZoneReply {
    ZoneReply::Bcm(BcmZoneReply {
        ctx: BcmContext {
            state: BcmState::Ready,
            left_turn_request: left,
            right_turn_request: right,
            ..Default::default()
        },
        outcomes: vec![],
        disposition: ObservationDisposition::Initial,
    })
}

async fn spawn_silent_bcm(
    identity: &str,
    transition_tx: Option<mpsc::Sender<crate::PublishedTransitionRecord>>,
) -> (VehicleController, ActorGuard<TwinMessage>) {
    let options = VehicleControllerRuntimeOptions {
        transition_tx,
        test_silent_bcm: true,
        test_silent_sccm: true,
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options(identity.to_owned(), options)
            .await
            .expect("spawn controller");
    let guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };
    (controller, guard)
}

#[tokio::test]
async fn bcm_become_on_tells_back_lifecycle_readiness() {
    let (bcm, _brain, mut rx, _bcm_guard, _brain_guard) = spawn_bcm_child(false).await;

    tell_bcm_zone(&bcm, 2, 0, BcmMessage::BecomeOn).expect("tell become on");
    let TwinMessage::ZoneReady {
        zone_id,
        turn_id,
        reply,
        ..
    } = next_ready(&mut rx).await
    else {
        panic!("expected ZoneReady");
    };

    assert_eq!(zone_id, AssemblyId::Bcm);
    assert_eq!(turn_id, 2);
    let bcm_reply = reply.as_bcm().expect("bcm reply");
    assert_eq!(bcm_reply.disposition, ObservationDisposition::Lifecycle);
    assert!(bcm_reply.outcomes.is_empty());
}

#[tokio::test]
async fn bcm_owns_independent_left_and_right_observations() {
    let (bcm, _brain, mut rx, _bcm_guard, _brain_guard) = spawn_bcm_child(false).await;

    tell_bcm_zone(&bcm, 4, 0, BcmMessage::LeftTurnRequestObserved(false)).expect("left");
    let TwinMessage::ZoneReady { reply, .. } = next_ready(&mut rx).await else {
        panic!("expected left ZoneReady");
    };
    let left = reply.as_bcm().expect("bcm reply");
    assert_eq!(left.disposition, ObservationDisposition::Initial);
    assert_eq!(left.ctx.left_turn_request, ObservedBool::Off);
    assert_eq!(left.ctx.right_turn_request, ObservedBool::Unknown);
    assert!(left.outcomes.is_empty());

    tell_bcm_zone(&bcm, 5, 0, BcmMessage::RightTurnRequestObserved(true)).expect("right");
    let TwinMessage::ZoneReady { reply, .. } = next_ready(&mut rx).await else {
        panic!("expected right ZoneReady");
    };
    let right = reply.as_bcm().expect("bcm reply");
    assert_eq!(right.disposition, ObservationDisposition::Initial);
    assert_eq!(right.ctx.left_turn_request, ObservedBool::Off);
    assert_eq!(right.ctx.right_turn_request, ObservedBool::On);
    assert!(
        !right
            .outcomes
            .iter()
            .any(|outcome| matches!(outcome, BcmOutcome::TurnLightsChanged { .. }))
    );
}

#[tokio::test]
async fn bcm_duplicate_left_does_not_touch_right_streak() {
    let (bcm, _brain, mut rx, _bcm_guard, _brain_guard) = spawn_bcm_child(false).await;

    tell_bcm_zone(&bcm, 1, 0, BcmMessage::RightTurnRequestObserved(false)).expect("right initial");
    let _ = next_ready(&mut rx).await;
    tell_bcm_zone(&bcm, 2, 0, BcmMessage::LeftTurnRequestObserved(false)).expect("left initial");
    let _ = next_ready(&mut rx).await;
    tell_bcm_zone(&bcm, 3, 0, BcmMessage::LeftTurnRequestObserved(false)).expect("left duplicate");
    let TwinMessage::ZoneReady { reply, .. } = next_ready(&mut rx).await else {
        panic!("expected duplicate ZoneReady");
    };
    let duplicate = reply.as_bcm().expect("bcm reply");
    assert_eq!(
        duplicate.disposition,
        ObservationDisposition::Duplicate {
            current_duplicates: 1
        }
    );
    assert_eq!(duplicate.ctx.left_turn_request, ObservedBool::Off);
    assert_eq!(duplicate.ctx.right_turn_request, ObservedBool::Off);
}

#[tokio::test]
async fn bcm_change_reports_completed_left_streak() {
    let (bcm, _brain, mut rx, _bcm_guard, _brain_guard) = spawn_bcm_child(false).await;

    for turn_id in 1..=3 {
        tell_bcm_zone(&bcm, turn_id, 0, BcmMessage::LeftTurnRequestObserved(false))
            .expect("left false");
        let _ = next_ready(&mut rx).await;
    }
    tell_bcm_zone(&bcm, 4, 0, BcmMessage::LeftTurnRequestObserved(true)).expect("left changed");
    let TwinMessage::ZoneReady { reply, .. } = next_ready(&mut rx).await else {
        panic!("expected changed ZoneReady");
    };
    let changed = reply.as_bcm().expect("bcm reply");
    assert_eq!(
        changed.disposition,
        ObservationDisposition::Changed {
            completed_duplicates: 2
        }
    );
    assert_eq!(changed.ctx.left_turn_request, ObservedBool::On);
}

#[tokio::test]
async fn bcm_shutdown_summaries_report_independent_remaining_streaks() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let (brain, _) = ractor::spawn::<ReadyCollector>(tx)
        .await
        .expect("collector");
    let mut state = BcmActorState::new(BcmContext::default(), false, brain.clone());
    state.left_streak.observe(false);
    state.left_streak.observe(false);
    state.right_streak.observe(true);

    assert_eq!(state.left_streak.pending_summary(), Some((false, 1)));
    assert_eq!(state.right_streak.pending_summary(), Some((true, 0)));
    brain.stop(None);
}

#[tokio::test]
async fn hazard_observation_does_not_compute_bcm_turn_lights() {
    let (tx, mut rx) = mpsc::channel(8);
    let options = VehicleControllerRuntimeOptions {
        transition_tx: Some(tx),
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("BCM-NO-COMPUTE".to_owned(), options)
            .await
            .expect("spawn controller");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    power_on_to_idle(&controller).await;
    let _power_on = rx.recv().await.expect("power on row");
    let _sccm_ready = rx.recv().await.expect("SCCM ready row");
    let _bcm_ready = rx.recv().await.expect("BCM ready row");
    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(true))
        .await
        .expect("hazard observation");
    let row = tokio::time::timeout(Duration::from_millis(250), rx.recv())
        .await
        .expect("hazard row timeout")
        .expect("transition channel closed");

    assert_eq!(row.event, PublishedFsmEvent::HazardButtonObserved(true));
    assert!(
        !row.actions
            .iter()
            .any(|action| matches!(action, PublishedDomainAction::SetTurnLights { .. }))
    );
    let snapshot = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(250)))
        .await
        .expect("snapshot");
    assert_eq!(snapshot.context().sccm.hazard_button, ObservedBool::On);
    assert_eq!(
        snapshot.context().bcm.left_turn_request,
        ObservedBool::Unknown
    );
    assert_eq!(
        snapshot.context().bcm.right_turn_request,
        ObservedBool::Unknown
    );
    let no_actions: &[DomainAction] = &[];
    assert!(
        !no_actions
            .iter()
            .any(|action| matches!(action, DomainAction::SetTurnLights { .. }))
    );
}

#[tokio::test]
async fn stale_or_wrong_bcm_tell_back_cannot_commit_observed_turn() {
    let (tx, mut rx) = mpsc::channel(8);
    let (controller, _guard) = spawn_silent_bcm("BCM-CORRELATION", Some(tx)).await;
    controller.send_power_on().await.expect("power on");
    let _power_on = rx.recv().await.expect("power on row");
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Sccm,
            turn_id: 2,
            tell_attempt: 0,
            reply: ZoneReply::Sccm(crate::vehicle_state::SccmZoneReply {
                ctx: Default::default(),
                disposition: ObservationDisposition::Lifecycle,
            }),
        })
        .expect("complete SCCM startup");
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 3,
            tell_attempt: 0,
            reply: ZoneReply::Bcm(BcmZoneReply {
                ctx: BcmContext {
                    state: BcmState::Ready,
                    ..Default::default()
                },
                outcomes: vec![],
                disposition: ObservationDisposition::Lifecycle,
            }),
        })
        .expect("complete BCM startup");
    wait_fsm_state(&controller, FsmState::Idle, Duration::from_millis(250)).await;
    let _sccm_ready = rx.recv().await.expect("sccm ready row");
    let _ready = rx.recv().await.expect("ready row");

    controller
        .submit_fsm_event(FsmEvent::LeftTurnRequestObserved(true))
        .await
        .expect("left ingress");
    tokio::task::yield_now().await;
    for (turn_id, tell_attempt) in [(99, 0), (4, 1)] {
        controller
            .get_actor_ref()
            .send_message(TwinMessage::ZoneReady {
                zone_id: AssemblyId::Bcm,
                turn_id,
                tell_attempt,
                reply: observed_reply(ObservedBool::On, ObservedBool::Unknown),
            })
            .expect("inject stale reply");
    }
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 4,
            tell_attempt: 0,
            reply: ZoneReply::Headlamp(HeadlampZoneReply {
                ctx: HeadlampContext::default(),
                outcomes: vec![],
            }),
        })
        .expect("inject wrong reply variant");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(rx.try_recv().is_err());
    assert_eq!(
        controller
            .get_snapshot(Some(ractor::concurrency::Duration::from_millis(50)))
            .await
            .expect("snapshot")
            .context()
            .bcm
            .left_turn_request,
        ObservedBool::Unknown
    );

    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 4,
            tell_attempt: 0,
            reply: observed_reply(ObservedBool::On, ObservedBool::Unknown),
        })
        .expect("inject correlated reply");
    let row = tokio::time::timeout(Duration::from_millis(250), rx.recv())
        .await
        .expect("correlated row timeout")
        .expect("transition channel closed");
    assert_eq!(row.event, PublishedFsmEvent::LeftTurnRequestObserved(true));
}

#[tokio::test]
async fn hazard_is_silent_at_actor_boundary_during_off_and_preparing_states() {
    let (tx, mut rx) = mpsc::channel(8);
    let (controller, _guard) = spawn_silent_bcm("BCM-GATE", Some(tx)).await;

    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(true))
        .await
        .expect("off hazard");
    controller.send_power_on().await.expect("power on");
    let _power_on = rx.recv().await.expect("power on row");
    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(true))
        .await
        .expect("preparing hazard");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(rx.try_recv().is_err());
    let snapshot = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(50)))
        .await
        .expect("snapshot");
    assert_eq!(snapshot.as_of_seq(), 1);
    assert_eq!(snapshot.context().sccm.hazard_button, ObservedBool::Unknown);
}

#[tokio::test]
async fn hazard_queued_during_shutdown_never_commits_after_assemblies_stop() {
    let (tx, mut rx) = mpsc::channel(8);
    let (controller, _guard) = spawn_silent_bcm("BCM-STOP-GATE", Some(tx)).await;
    controller.send_power_on().await.expect("power on");
    let _power_on = rx.recv().await.expect("power on row");
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Sccm,
            turn_id: 2,
            tell_attempt: 0,
            reply: ZoneReply::Sccm(crate::vehicle_state::SccmZoneReply {
                ctx: Default::default(),
                disposition: ObservationDisposition::Lifecycle,
            }),
        })
        .expect("SCCM startup reply");
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 3,
            tell_attempt: 0,
            reply: ZoneReply::Bcm(BcmZoneReply {
                ctx: BcmContext {
                    state: BcmState::Ready,
                    ..Default::default()
                },
                outcomes: vec![],
                disposition: ObservationDisposition::Lifecycle,
            }),
        })
        .expect("startup reply");
    wait_fsm_state(&controller, FsmState::Idle, Duration::from_millis(250)).await;
    let _sccm_ready = rx.recv().await.expect("sccm ready row");
    let _ready = rx.recv().await.expect("ready row");

    controller.send_power_off().await.expect("power off");
    wait_fsm_state(
        &controller,
        FsmState::PreparingToStop(std::collections::BTreeSet::from([
            AssemblyId::Sccm,
            AssemblyId::Bcm,
        ])),
        Duration::from_millis(250),
    )
    .await;
    let _power_off = rx.recv().await.expect("power off row");
    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(true))
        .await
        .expect("hazard during shutdown");
    tokio::task::yield_now().await;
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Sccm,
            turn_id: 5,
            tell_attempt: 0,
            reply: ZoneReply::Sccm(crate::vehicle_state::SccmZoneReply {
                ctx: Default::default(),
                disposition: ObservationDisposition::Lifecycle,
            }),
        })
        .expect("SCCM shutdown reply");
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 6,
            tell_attempt: 0,
            reply: ZoneReply::Bcm(BcmZoneReply {
                ctx: BcmContext::default(),
                outcomes: vec![],
                disposition: ObservationDisposition::Lifecycle,
            }),
        })
        .expect("shutdown reply");
    wait_fsm_state(&controller, FsmState::Off, Duration::from_millis(250)).await;
    let _sccm_stopped = rx.recv().await.expect("SCCM stopped row");
    let stopped = rx.recv().await.expect("stopped row");
    assert_eq!(stopped.next_state, crate::PublishedFsmState::Off);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(rx.try_recv().is_err(), "hazard must not leave a queued row");
    let snapshot = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(50)))
        .await
        .expect("snapshot");
    assert_eq!(snapshot.context().sccm.hazard_button, ObservedBool::Unknown);
}
