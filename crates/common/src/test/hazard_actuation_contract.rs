use std::time::Duration;

use tokio::sync::mpsc;

use crate::digital_twin::DigitalTwinCar;
use crate::fsm::{DomainAction, FsmEvent, FsmState};
use crate::test::{ActorGuard, power_on_to_idle};
use crate::twin_runtime::controller::actuation_manager::{
    ActuationManager, DefaultActuationManager,
};
use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;
use crate::vehicle_state::VehicleContext;
use crate::{ActuationCommand, CorrelationId, PublishedFsmEvent, VehicleController};

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
    let _bcm_ready = transition_rx.recv().await.expect("BCM-ready record");

    controller
        .submit_fsm_event(FsmEvent::HazardButtonChanged(true))
        .await
        .expect("hazard on");
    let on_record = tokio::time::timeout(Duration::from_millis(500), transition_rx.recv())
        .await
        .expect("hazard-on record timeout")
        .expect("transition channel closed");
    assert_eq!(
        on_record.event,
        PublishedFsmEvent::HazardButtonChanged(true)
    );
    let on_command = tokio::time::timeout(Duration::from_millis(500), actuation_rx.recv())
        .await
        .expect("hazard-on command timeout")
        .expect("actuation channel closed");
    assert_eq!(
        on_command,
        ActuationCommand::SetTurnLights {
            correlation_id: CorrelationId {
                source_id: "HAZARD-ACTUATION".into(),
                session_id: match &on_command {
                    ActuationCommand::SetTurnLights { correlation_id, .. } => {
                        correlation_id.session_id
                    }
                    _ => unreachable!(),
                },
                sequence_no: 1,
            },
            left_on: true,
            right_on: true,
        }
    );

    controller
        .submit_fsm_event(FsmEvent::HazardButtonChanged(true))
        .await
        .expect("duplicate hazard on");
    let duplicate_record = tokio::time::timeout(Duration::from_millis(500), transition_rx.recv())
        .await
        .expect("duplicate hazard record timeout")
        .expect("transition channel closed");
    assert_eq!(
        duplicate_record.event,
        PublishedFsmEvent::HazardButtonChanged(true)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), actuation_rx.recv())
            .await
            .is_err(),
        "duplicate hazard input must not emit a command"
    );

    controller
        .submit_fsm_event(FsmEvent::HazardButtonChanged(false))
        .await
        .expect("hazard off");
    let off_record = tokio::time::timeout(Duration::from_millis(500), transition_rx.recv())
        .await
        .expect("hazard-off record timeout")
        .expect("transition channel closed");
    assert_eq!(
        off_record.event,
        PublishedFsmEvent::HazardButtonChanged(false)
    );
    let off_command = tokio::time::timeout(Duration::from_millis(500), actuation_rx.recv())
        .await
        .expect("hazard-off command timeout")
        .expect("actuation channel closed");
    assert!(matches!(
        off_command,
        ActuationCommand::SetTurnLights {
            correlation_id: CorrelationId { sequence_no: 2, .. },
            left_on: false,
            right_on: false,
        }
    ));
}
