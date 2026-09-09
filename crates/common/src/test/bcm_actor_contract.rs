use std::time::Duration;

use tokio::sync::mpsc;

use crate::digital_twin::{TwinMessage, ZoneReply};
use crate::fsm::{AssemblyId, FsmEvent, FsmState};
use crate::test::{ActorGuard, power_on_to_idle, wait_fsm_state};
use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;
use crate::vehicle_state::{
    BcmContext, BcmOutcome, BcmState, BcmZoneReply, HeadlampContext, HeadlampZoneReply,
};
use crate::{PublishedFsmEvent, VehicleController};

fn hazard_reply(on: bool) -> ZoneReply {
    ZoneReply::Bcm(BcmZoneReply {
        ctx: BcmContext {
            state: BcmState::Ready,
            left_turn_request_on: on,
            right_turn_request_on: on,
        },
        outcomes: vec![BcmOutcome::TurnLightsChanged {
            left_on: on,
            right_on: on,
        }],
    })
}

async fn spawn_silent_bcm(
    identity: &str,
    transition_tx: Option<mpsc::Sender<crate::PublishedTransitionRecord>>,
) -> (VehicleController, ActorGuard<TwinMessage>) {
    let options = VehicleControllerRuntimeOptions {
        transition_tx,
        test_silent_bcm: true,
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
async fn bcm_tell_back_commits_correlated_hazard_state_and_record() {
    let (tx, mut rx) = mpsc::channel(8);
    let options = VehicleControllerRuntimeOptions {
        transition_tx: Some(tx),
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("BCM-ACTOR".to_owned(), options)
            .await
            .expect("spawn controller");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    power_on_to_idle(&controller).await;
    let _power_on = rx.recv().await.expect("power on row");
    let _bcm_ready = rx.recv().await.expect("BCM ready row");
    controller
        .submit_fsm_event(FsmEvent::HazardButtonChanged(true))
        .await
        .expect("hazard ingress");
    let row = tokio::time::timeout(Duration::from_millis(250), rx.recv())
        .await
        .expect("hazard row timeout")
        .expect("transition channel closed");

    assert_eq!(row.event, PublishedFsmEvent::HazardButtonChanged(true));
    let snapshot = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(250)))
        .await
        .expect("snapshot");
    assert!(snapshot.context().sccm.hazard_button_on);
    assert!(snapshot.context().bcm.right_turn_request_on);
}

#[tokio::test]
async fn stale_or_wrong_bcm_tell_back_cannot_commit_hazard_turn() {
    let (tx, mut rx) = mpsc::channel(8);
    let (controller, _guard) = spawn_silent_bcm("BCM-CORRELATION", Some(tx)).await;
    controller.send_power_on().await.expect("power on");
    let _power_on = rx.recv().await.expect("power on row");
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 2,
            tell_attempt: 0,
            reply: ZoneReply::Bcm(BcmZoneReply {
                ctx: BcmContext {
                    state: BcmState::Ready,
                    ..Default::default()
                },
                outcomes: vec![],
            }),
        })
        .expect("complete startup");
    wait_fsm_state(&controller, FsmState::Idle, Duration::from_millis(250)).await;
    let _ready = rx.recv().await.expect("ready row");

    controller
        .submit_fsm_event(FsmEvent::HazardButtonChanged(true))
        .await
        .expect("hazard ingress");
    tokio::task::yield_now().await;
    for (turn_id, tell_attempt) in [(99, 0), (3, 1)] {
        controller
            .get_actor_ref()
            .send_message(TwinMessage::ZoneReady {
                zone_id: AssemblyId::Bcm,
                turn_id,
                tell_attempt,
                reply: hazard_reply(true),
            })
            .expect("inject stale reply");
    }
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 3,
            tell_attempt: 0,
            reply: ZoneReply::Headlamp(HeadlampZoneReply {
                ctx: HeadlampContext::default(),
                outcomes: vec![],
            }),
        })
        .expect("inject wrong reply variant");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(rx.try_recv().is_err());
    assert!(
        !controller
            .get_snapshot(Some(ractor::concurrency::Duration::from_millis(50)))
            .await
            .expect("snapshot")
            .context()
            .sccm
            .hazard_button_on
    );

    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 3,
            tell_attempt: 0,
            reply: hazard_reply(true),
        })
        .expect("inject correlated reply");
    let row = tokio::time::timeout(Duration::from_millis(250), rx.recv())
        .await
        .expect("correlated row timeout")
        .expect("transition channel closed");
    assert_eq!(row.event, PublishedFsmEvent::HazardButtonChanged(true));
}

#[tokio::test]
async fn hazard_is_silent_at_actor_boundary_during_off_and_preparing_states() {
    let (tx, mut rx) = mpsc::channel(8);
    let (controller, _guard) = spawn_silent_bcm("BCM-GATE", Some(tx)).await;

    controller
        .submit_fsm_event(FsmEvent::HazardButtonChanged(true))
        .await
        .expect("off hazard");
    controller.send_power_on().await.expect("power on");
    let _power_on = rx.recv().await.expect("power on row");
    controller
        .submit_fsm_event(FsmEvent::HazardButtonChanged(true))
        .await
        .expect("preparing hazard");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(rx.try_recv().is_err());
    let snapshot = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(50)))
        .await
        .expect("snapshot");
    assert_eq!(snapshot.as_of_seq(), 1);
    assert!(!snapshot.context().sccm.hazard_button_on);
}

#[tokio::test]
async fn hazard_queued_during_shutdown_never_commits_after_bcm_stops() {
    let (tx, mut rx) = mpsc::channel(8);
    let (controller, _guard) = spawn_silent_bcm("BCM-STOP-GATE", Some(tx)).await;
    controller.send_power_on().await.expect("power on");
    let _power_on = rx.recv().await.expect("power on row");
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 2,
            tell_attempt: 0,
            reply: ZoneReply::Bcm(BcmZoneReply {
                ctx: BcmContext {
                    state: BcmState::Ready,
                    ..Default::default()
                },
                outcomes: vec![],
            }),
        })
        .expect("startup reply");
    wait_fsm_state(&controller, FsmState::Idle, Duration::from_millis(250)).await;
    let _ready = rx.recv().await.expect("ready row");

    controller.send_power_off().await.expect("power off");
    wait_fsm_state(
        &controller,
        FsmState::PreparingToStop(std::collections::BTreeSet::from([AssemblyId::Bcm])),
        Duration::from_millis(250),
    )
    .await;
    let _power_off = rx.recv().await.expect("power off row");
    controller
        .submit_fsm_event(FsmEvent::HazardButtonChanged(true))
        .await
        .expect("hazard during shutdown");
    tokio::task::yield_now().await;
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 4,
            tell_attempt: 0,
            reply: ZoneReply::Bcm(BcmZoneReply {
                ctx: BcmContext::default(),
                outcomes: vec![],
            }),
        })
        .expect("shutdown reply");
    wait_fsm_state(&controller, FsmState::Off, Duration::from_millis(250)).await;
    let stopped = rx.recv().await.expect("stopped row");
    assert_eq!(stopped.next_state, crate::PublishedFsmState::Off);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(rx.try_recv().is_err(), "hazard must not leave a queued row");
    let snapshot = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(50)))
        .await
        .expect("snapshot");
    assert!(!snapshot.context().sccm.hazard_button_on);
}
