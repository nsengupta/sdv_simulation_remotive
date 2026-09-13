use std::time::Duration;

use tokio::sync::mpsc;

use crate::digital_twin::DigitalTwinCar;
use crate::fsm::{DomainAction, FsmEvent, FsmState};
use crate::test::{ActorGuard, power_on_to_idle};
use crate::twin_runtime::controller::actuation_manager::{
    ActuationError, ActuationManager, DefaultActuationManager,
};
use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;
use crate::vehicle_state::VehicleContext;
use crate::{
    ActuationCommand, CorrelationId, DiagnosticKind, PublishedFsmEvent, PublishedObservedBool,
    VehicleController, VehicleControllerError,
};

fn blank_twin() -> DigitalTwinCar {
    DigitalTwinCar::new("hazard-actuation", FsmState::Off, VehicleContext::default())
        .expect("test twin must be constructible")
}

#[tokio::test]
async fn set_turn_lights_maps_to_one_atomic_command_with_scoped_correlation() {
    let (tx, mut rx) = mpsc::channel(4);
    let manager = DefaultActuationManager::with_command_channel("hazard-source".into(), 41, tx);

    manager
        .execute(
            &DomainAction::SetTurnLights {
                left_on: true,
                right_on: false,
            },
            &blank_twin(),
        )
        .await
        .expect("execute turn-light action");

    assert_eq!(
        rx.recv().await,
        Some(ActuationCommand::SetTurnLights {
            correlation_id: CorrelationId {
                source_id: "hazard-source".into(),
                session_id: 41,
                sequence_no: 1,
            },
            left_on: true,
            right_on: false,
        })
    );
}

#[tokio::test]
async fn closed_actuation_command_channel_is_reported_as_an_actuation_error() {
    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    let manager = DefaultActuationManager::with_command_channel("closed-channel".into(), 7, tx);

    let error = manager
        .execute(
            &DomainAction::SetTurnLights {
                left_on: true,
                right_on: true,
            },
            &blank_twin(),
        )
        .await
        .expect_err("closed command channel must not be reported as success");

    assert_eq!(
        error,
        ActuationError::CommandChannelClosed("set_turn_lights")
    );
}

#[tokio::test]
async fn changed_hazard_commands_are_deduplicated_without_consuming_correlation_sequence() {
    let (transition_tx, mut transition_rx) = mpsc::channel(8);
    let (actuation_tx, mut actuation_rx) = mpsc::channel(8);
    let options = VehicleControllerRuntimeOptions {
        transition_tx: Some(transition_tx),
        actuation_command_tx: Some(actuation_tx),
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("HAZARD-ACTUATION".to_string(), options)
            .await
            .expect("install controller");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    power_on_to_idle(&controller).await;
    let _power_on = transition_rx.recv().await.expect("power-on record");
    let _sccm_ready = transition_rx.recv().await.expect("SCCM-ready record");
    let _bcm_ready = transition_rx.recv().await.expect("BCM-ready record");

    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(true))
        .await
        .expect("hazard on");
    let on_record = tokio::time::timeout(Duration::from_millis(500), transition_rx.recv())
        .await
        .expect("hazard-on record timeout")
        .expect("transition channel closed");
    assert_eq!(
        on_record.event,
        PublishedFsmEvent::HazardButtonObserved(true)
    );
    assert_eq!(
        on_record.current_ctx.sccm.hazard_button_on,
        PublishedObservedBool::On,
        "published ledger must mirror the observed SCCM value"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), actuation_rx.recv())
            .await
            .is_err(),
        "observed hazard must not emit SetTurnLights"
    );

    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(true))
        .await
        .expect("duplicate hazard on");
    assert!(
        tokio::time::timeout(Duration::from_millis(50), transition_rx.recv())
            .await
            .is_err(),
        "duplicate observed hazard must not emit a record"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), actuation_rx.recv())
            .await
            .is_err(),
        "duplicate hazard input must not emit a command"
    );

    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(false))
        .await
        .expect("hazard off");
    let off_record = tokio::time::timeout(Duration::from_millis(500), transition_rx.recv())
        .await
        .expect("hazard-off record timeout")
        .expect("transition channel closed");
    assert_eq!(
        off_record.event,
        PublishedFsmEvent::HazardButtonObserved(false)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), actuation_rx.recv())
            .await
            .is_err(),
        "observed hazard off must not emit SetTurnLights"
    );
}

#[tokio::test]
async fn saturated_transition_channel_does_not_retain_duplicate_observed_hazard_rows() {
    const DUPLICATES: usize = 32;
    let (transition_tx, mut transition_rx) = mpsc::channel(8);
    let options = VehicleControllerRuntimeOptions {
        transition_tx: Some(transition_tx),
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("HAZARD-SATURATION".to_string(), options)
            .await
            .expect("install controller");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    power_on_to_idle(&controller).await;
    transition_rx.recv().await.expect("power-on record");
    transition_rx.recv().await.expect("SCCM-ready record");
    transition_rx.recv().await.expect("BCM-ready record");

    for _ in 0..DUPLICATES {
        controller
            .submit_fsm_event(FsmEvent::HazardButtonObserved(true))
            .await
            .expect("duplicate hazard accepted");
    }

    let initial = tokio::time::timeout(Duration::from_millis(500), transition_rx.recv())
        .await
        .expect("initial observed record timeout")
        .expect("transition channel closed");
    assert_eq!(initial.event, PublishedFsmEvent::HazardButtonObserved(true));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), transition_rx.recv())
            .await
            .is_err(),
        "duplicate observed hazards must not leave ledger rows"
    );
}

#[tokio::test]
async fn stalled_transition_consumer_backpressures_actor_until_capacity_is_released() {
    let (transition_tx, mut transition_rx) = mpsc::channel(2);
    let options = VehicleControllerRuntimeOptions {
        transition_tx: Some(transition_tx),
        ..Default::default()
    };
    let (controller, handle) = VehicleController::install_and_start_with_options(
        "HAZARD-BACKPRESSURE".to_string(),
        options,
    )
    .await
    .expect("install controller");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    controller.send_power_on().await.expect("power on");
    transition_rx.recv().await.expect("power-on record");
    transition_rx.recv().await.expect("SCCM-ready record");
    transition_rx.recv().await.expect("BCM-ready record");
    crate::test::wait_fsm_state(&controller, FsmState::Idle, Duration::from_millis(250)).await;

    for event in [
        FsmEvent::HazardButtonObserved(true),
        FsmEvent::HazardButtonObserved(false),
        FsmEvent::HazardButtonObserved(true),
    ] {
        controller
            .submit_fsm_event(event)
            .await
            .expect("submit changed hazard");
    }
    tokio::task::yield_now().await;

    assert!(
        matches!(
            controller
                .get_snapshot(Some(Duration::from_millis(50)))
                .await,
            Err(VehicleControllerError::Timeout)
        ),
        "a detached queue incorrectly lets state advance past bounded record capacity"
    );

    let first = transition_rx.recv().await.expect("first changed record");
    let snapshot = controller
        .get_snapshot(Some(Duration::from_millis(250)))
        .await
        .expect("actor must resume once bounded capacity is released");
    assert_eq!(snapshot.as_of_seq(), 6);

    let second = transition_rx.recv().await.expect("second changed record");
    let third = transition_rx.recv().await.expect("third changed record");
    assert_eq!(second.record_seq, first.record_seq + 1);
    assert_eq!(third.record_seq, second.record_seq + 1);
}

#[tokio::test]
async fn closed_transition_channel_reports_error_and_prevents_state_commit() {
    let (transition_tx, transition_rx) = mpsc::channel(1);
    drop(transition_rx);
    let (diagnostic_tx, mut diagnostic_rx) = mpsc::unbounded_channel();
    let options = VehicleControllerRuntimeOptions {
        transition_tx: Some(transition_tx),
        diagnostic_tx: Some(diagnostic_tx),
        ..Default::default()
    };
    let (controller, handle) = VehicleController::install_and_start_with_options(
        "HAZARD-CLOSED-TRANSITION".to_string(),
        options,
    )
    .await
    .expect("install controller");

    controller.send_power_on().await.expect("power on accepted");

    let diagnostic = tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            let record = diagnostic_rx
                .recv()
                .await
                .expect("diagnostic channel closed");
            if matches!(record.kind, DiagnosticKind::TransitionSinkClosed) {
                break record;
            }
        }
    })
    .await
    .expect("transition channel closure was not diagnosed");
    assert!(matches!(
        diagnostic.kind,
        DiagnosticKind::TransitionSinkClosed
    ));

    tokio::time::timeout(Duration::from_millis(250), handle)
        .await
        .expect("actor did not stop after commit admission failed")
        .expect("actor join failed");
    assert!(
        controller
            .get_snapshot(Some(Duration::from_millis(50)))
            .await
            .is_err(),
        "actor must not remain available with unrecorded committed state"
    );
}
