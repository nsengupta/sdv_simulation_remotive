//! Contract: when the wiper twinlet never replies (simulated by `test_silent_wiper`),
//! the tell-back timeout exhausts, a synthetic `WiperOutcome::LogWarning` is emitted,
//! and the actor's diagnostic stream receives a warning message.

use std::time::Duration;

use tokio::sync::mpsc;

use crate::VehicleController;
use crate::digital_twin::TwinMessage;
use crate::fsm::FsmState;
use crate::observation_records::diagnostic::DiagnosticRecord;
use crate::test::{ActorGuard, wait_fsm_state};
use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;

/// Drain the diagnostic channel, returning all messages received within `window`.
async fn drain_diagnostics(
    rx: &mut mpsc::UnboundedReceiver<DiagnosticRecord>,
    window: Duration,
) -> Vec<DiagnosticRecord> {
    let mut msgs = vec![];
    let deadline = tokio::time::Instant::now() + window;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(m)) => msgs.push(m),
            _ => break,
        }
    }
    msgs
}

#[tokio::test]
async fn given_silent_wiper_when_powering_on_then_bcm_only_startup_reaches_idle_without_warning() {
    let (diag_tx, mut diag_rx) = mpsc::unbounded_channel::<DiagnosticRecord>();
    let opts = VehicleControllerRuntimeOptions {
        diagnostic_tx: Some(diag_tx),
        test_silent_wiper: true,
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("WIPER-FAIL-01".to_string(), opts)
            .await
            .expect("install actor");
    let _guard = ActorGuard::<TwinMessage> {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    // Wiper is retained for compatibility but is not a lifecycle participant.
    controller.send_power_on().await.expect("power on");

    wait_fsm_state(&controller, FsmState::Idle, Duration::from_millis(500)).await;

    // Collect all diagnostics that arrived by now.
    let messages = drain_diagnostics(&mut diag_rx, Duration::from_millis(50)).await;

    let has_wiper_warning = messages.iter().any(|m| {
        m.level == crate::observation_records::diagnostic::DiagnosticLevel::Warning
            && matches!(
                &m.kind,
                crate::DiagnosticKind::Text { text } if text.to_lowercase().contains("wiper")
            )
    });
    assert!(
        !has_wiper_warning,
        "wiper must not join startup: {messages:#?}"
    );
}
