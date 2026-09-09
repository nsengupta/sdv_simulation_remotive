//! Wiper actuation path contract tests.
//!
//! Covers Steps 2–5 and Step 9 (end-to-end rain ingress) of the Wiper implementation plan.

use std::time::Duration;

use crate::DiagnosticKind;
use crate::VehicleController;
use crate::digital_twin::DigitalTwinCar;
use crate::fsm::{DomainAction, FsmEvent, FsmState};
use crate::test::ActorGuard;
use crate::test::{
    expect_actuation_command, install_with_actuation, power_on_to_idle,
    wiper_zone_contract::wait_wiper_state,
};
use crate::twin_runtime::controller::actuation_contract::ActuationCommand;
use crate::twin_runtime::controller::actuation_manager::{
    ActuationManager, DefaultActuationManager,
};
use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;
use crate::twin_runtime::outcome_map::zone_outcomes_to_domain_actions;
use crate::twin_runtime::zone_turn::ZoneOutcome;
use crate::vehicle_state::WiperOutcome;
use crate::vehicle_state::{VehicleContext, WiperState};
use tokio::sync::mpsc;

// ── Step 2: DomainAction variants ─────────────────────────────────────────────

#[test]
fn given_request_wiper_actions_when_compared_then_distinct() {
    assert_ne!(
        format!("{:?}", DomainAction::RequestWiperStart),
        format!("{:?}", DomainAction::RequestWiperStop),
        "RequestWiperStart and RequestWiperStop must be distinct variants"
    );
}

// ── Step 3: outcome_map wiper path ────────────────────────────────────────────

#[test]
fn given_start_wiping_outcome_when_mapped_then_request_wiper_start() {
    let outcomes = vec![ZoneOutcome::Wiper(WiperOutcome::StartWiping)];
    let actions = zone_outcomes_to_domain_actions(outcomes);
    assert_eq!(actions, vec![DomainAction::RequestWiperStart]);
}

#[test]
fn given_stop_wiping_outcome_when_mapped_then_request_wiper_stop() {
    let outcomes = vec![ZoneOutcome::Wiper(WiperOutcome::StopWiping)];
    let actions = zone_outcomes_to_domain_actions(outcomes);
    assert_eq!(actions, vec![DomainAction::RequestWiperStop]);
}

#[test]
fn given_wiper_log_warning_outcome_when_mapped_then_domain_log_warning() {
    let msg = "wiper unresponsive".to_string();
    let outcomes = vec![ZoneOutcome::Wiper(WiperOutcome::LogWarning(msg.clone()))];
    let actions = zone_outcomes_to_domain_actions(outcomes);
    assert_eq!(actions, vec![DomainAction::LogWarning(msg)]);
}

// ── Step 5: actuation_manager sends commands on channel ───────────────────────

fn blank_twin() -> DigitalTwinCar {
    DigitalTwinCar::new("test-wiper", FsmState::Off, VehicleContext::default())
        .expect("test twin must be constructible")
}

#[tokio::test]
async fn given_request_wiper_start_when_actuation_manager_executes_then_sends_start_wiper() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ActuationCommand>(4);
    let mgr = DefaultActuationManager::with_command_channel("test".into(), 0, tx);
    mgr.execute(&DomainAction::RequestWiperStart, &blank_twin())
        .await
        .expect("execute must not fail");
    let cmd = rx.try_recv().expect("StartWiper command must be sent");
    assert!(matches!(cmd, ActuationCommand::StartWiper));
}

#[tokio::test]
async fn given_request_wiper_stop_when_actuation_manager_executes_then_sends_stop_wiper() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ActuationCommand>(4);
    let mgr = DefaultActuationManager::with_command_channel("test".into(), 0, tx);
    mgr.execute(&DomainAction::RequestWiperStop, &blank_twin())
        .await
        .expect("execute must not fail");
    let cmd = rx.try_recv().expect("StopWiper command must be sent");
    assert!(matches!(cmd, ActuationCommand::StopWiper));
}

// ── Step 4: ActuationCommand variants ─────────────────────────────────────────

#[test]
fn given_wiper_actuation_commands_when_compared_then_distinct() {
    assert_ne!(
        format!("{:?}", ActuationCommand::StartWiper),
        format!("{:?}", ActuationCommand::StopWiper),
        "StartWiper and StopWiper must be distinct variants"
    );
}

// ── Step 9: end-to-end physical rain ingress ──────────────────────────────────

#[tokio::test]
async fn given_idle_wiper_ready_when_rain_detected_true_ingress_then_running_and_start_wiper_command()
 {
    let (controller, mut actuation_rx, _guard) = install_with_actuation("WIPER-E2E-1", 8).await;
    power_on_to_idle(&controller).await;

    controller
        .submit_fsm_event(FsmEvent::RainsStarted)
        .await
        .expect("rain ingress");

    wait_wiper_state(&controller, WiperState::Running, Duration::from_millis(500)).await;

    let cmd = expect_actuation_command(&mut actuation_rx, Duration::from_secs(1)).await;
    assert!(matches!(cmd, ActuationCommand::StartWiper), "got {cmd:?}");
}

#[tokio::test]
async fn given_wiper_running_when_rain_detected_false_ingress_then_ready_and_stop_wiper_command() {
    let (controller, mut actuation_rx, _guard) = install_with_actuation("WIPER-E2E-2", 8).await;
    power_on_to_idle(&controller).await;

    controller
        .submit_fsm_event(FsmEvent::RainsStarted)
        .await
        .expect("start rain");
    let _ = expect_actuation_command(&mut actuation_rx, Duration::from_secs(1)).await;
    wait_wiper_state(&controller, WiperState::Running, Duration::from_millis(500)).await;

    controller
        .submit_fsm_event(FsmEvent::RainsStopped)
        .await
        .expect("stop rain");

    wait_wiper_state(&controller, WiperState::Ready, Duration::from_millis(500)).await;
    let cmd = expect_actuation_command(&mut actuation_rx, Duration::from_secs(1)).await;
    assert!(matches!(cmd, ActuationCommand::StopWiper), "got {cmd:?}");
}

#[tokio::test]
async fn given_rain_ingress_when_wiper_runs_then_diagnostics_prove_rain_wiper_coupling() {
    let (diag_tx, mut diag_rx) = mpsc::unbounded_channel();
    let (actuation_tx, mut actuation_rx) = mpsc::channel(8);
    let runtime_options = VehicleControllerRuntimeOptions {
        diagnostic_tx: Some(diag_tx),
        actuation_command_tx: Some(actuation_tx),
        ..Default::default()
    };
    let (controller, handle) = VehicleController::install_and_start_with_options(
        "WIPER-DIAG-RAIN".to_string(),
        runtime_options,
    )
    .await
    .expect("install");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    power_on_to_idle(&controller).await;
    while diag_rx.try_recv().is_ok() {}

    controller
        .submit_fsm_event(FsmEvent::RainsStarted)
        .await
        .expect("rain");
    wait_wiper_state(&controller, WiperState::Running, Duration::from_millis(500)).await;
    let _ = expect_actuation_command(&mut actuation_rx, Duration::from_secs(1)).await;

    let mut saw_rain = false;
    let mut saw_wiper = false;
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(200), diag_rx.recv()).await
    {
        match msg.kind {
            DiagnosticKind::RainChanged { raining: true } => saw_rain = true,
            DiagnosticKind::WiperMotionChanged { wiping: true } => saw_wiper = true,
            _ => {}
        }
    }
    assert!(saw_rain, "expected RainChanged {{ raining: true }}");
    assert!(saw_wiper, "expected WiperMotionChanged {{ wiping: true }}");

    controller
        .submit_fsm_event(FsmEvent::RainsStopped)
        .await
        .expect("rain stop");
    wait_wiper_state(&controller, WiperState::Ready, Duration::from_millis(500)).await;

    let mut saw_rain_off = false;
    let mut saw_wiper_off = false;
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(200), diag_rx.recv()).await
    {
        match msg.kind {
            DiagnosticKind::RainChanged { raining: false } => saw_rain_off = true,
            DiagnosticKind::WiperMotionChanged { wiping: false } => saw_wiper_off = true,
            _ => {}
        }
    }
    assert!(saw_rain_off, "expected RainChanged {{ raining: false }}");
    assert!(
        saw_wiper_off,
        "expected WiperMotionChanged {{ wiping: false }}"
    );
}
