//! Delayed SCCM tell-back must not let a later BCM observation commit first.

use std::time::Duration;

use tokio::sync::mpsc;

use crate::digital_twin::{TwinMessage, ZoneReply};
use crate::fsm::{AssemblyId, FsmEvent, FsmState};
use crate::test::{ActorGuard, wait_fsm_state};
use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;
use crate::vehicle_state::{
    BcmContext, BcmState, BcmZoneReply, ObservationDisposition, ObservedBool, SccmZoneReply,
};
use crate::{PublishedFsmEvent, PublishedTransitionRecord, VehicleController};

async fn recv_row(rx: &mut mpsc::Receiver<PublishedTransitionRecord>) -> PublishedTransitionRecord {
    tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .expect("transition row timeout")
        .expect("transition channel closed")
}

#[tokio::test]
async fn delayed_sccm_reply_keeps_ingress_commit_order() {
    let (tx, mut rx) = mpsc::channel(16);
    let options = VehicleControllerRuntimeOptions {
        transition_tx: Some(tx),
        test_silent_sccm: true,
        test_silent_bcm: true,
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("OBS-REORDER".to_owned(), options)
            .await
            .expect("spawn controller");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    controller.send_power_on().await.expect("power on");
    let _power_on = recv_row(&mut rx).await;
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Sccm,
            turn_id: 2,
            tell_attempt: 0,
            reply: ZoneReply::Sccm(SccmZoneReply {
                ctx: Default::default(),
                disposition: ObservationDisposition::Lifecycle,
            }),
        })
        .expect("sccm ready");
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
        .expect("bcm ready");
    wait_fsm_state(&controller, FsmState::Idle, Duration::from_millis(250)).await;
    let _sccm_ready = recv_row(&mut rx).await;
    let _bcm_ready = recv_row(&mut rx).await;

    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(false))
        .await
        .expect("earlier sccm turn");
    controller
        .submit_fsm_event(FsmEvent::LeftTurnRequestObserved(true))
        .await
        .expect("later bcm turn");
    tokio::task::yield_now().await;

    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Bcm,
            turn_id: 5,
            tell_attempt: 0,
            reply: ZoneReply::Bcm(BcmZoneReply {
                ctx: BcmContext {
                    state: BcmState::Ready,
                    left_turn_request: ObservedBool::On,
                    ..Default::default()
                },
                outcomes: vec![],
                disposition: ObservationDisposition::Initial,
            }),
        })
        .expect("later bcm reply first");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        rx.try_recv().is_err(),
        "later BCM reply must wait for the earlier SCCM head-of-buffer"
    );

    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Sccm,
            turn_id: 4,
            tell_attempt: 0,
            reply: ZoneReply::Sccm(SccmZoneReply {
                ctx: crate::vehicle_state::SccmContext {
                    hazard_button: ObservedBool::Off,
                    ..Default::default()
                },
                disposition: ObservationDisposition::Initial,
            }),
        })
        .expect("earlier sccm reply");

    let first = recv_row(&mut rx).await;
    let second = recv_row(&mut rx).await;
    assert_eq!(first.event, PublishedFsmEvent::HazardButtonChanged(false));
    assert_eq!(second.event, PublishedFsmEvent::TimerTick);
    assert_eq!(second.record_seq, first.record_seq + 1);

    let snapshot = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(250)))
        .await
        .expect("snapshot");
    assert_eq!(snapshot.context().sccm.hazard_button, ObservedBool::Off);
    assert_eq!(snapshot.context().bcm.left_turn_request, ObservedBool::On);
}
