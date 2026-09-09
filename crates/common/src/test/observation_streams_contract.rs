//! Twin-side observation stream contracts — ledger and diagnostic channels without UI.
//!
//! Verifies both streams share [`SessionClock`] timing and can be tested by wiring
//! `VehicleController` to tokio mpsc receivers only (no Dashboard, no Gateway tick).

use std::time::Duration;

use tokio::sync::mpsc;

use crate::VehicleController;
use crate::observation_records::diagnostic::{DiagnosticRecord, elapsed_since_session};
use crate::test::{ActorGuard, power_on_to_idle};
use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;

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
async fn given_twin_boot_when_both_sinks_wired_then_ledger_and_diagnostic_share_session_start() {
    let (diag_tx, mut diag_rx) = mpsc::unbounded_channel();
    let (trans_tx, mut trans_rx) = mpsc::channel(16);

    let runtime_options = VehicleControllerRuntimeOptions {
        diagnostic_tx: Some(diag_tx),
        transition_tx: Some(trans_tx),
        ..VehicleControllerRuntimeOptions::default()
    };

    let (controller, handle) = VehicleController::install_and_start_with_options(
        "OBS-STREAMS-01".to_string(),
        runtime_options,
    )
    .await
    .expect("start actor");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    power_on_to_idle(&controller).await;

    let mut ledger_rows = vec![];
    while let Ok(row) = trans_rx.try_recv() {
        ledger_rows.push(row);
    }
    assert!(
        ledger_rows.len() >= 2,
        "expected boot ledger rows, got {}",
        ledger_rows.len()
    );

    let diagnostics = drain_diagnostics(&mut diag_rx, Duration::from_millis(100)).await;
    assert!(!diagnostics.is_empty(), "expected at least boot diagnostic");

    let session = ledger_rows[0].session_started_at;
    assert!(
        ledger_rows.iter().all(|r| r.session_started_at == session),
        "ledger rows must share one session anchor"
    );
    assert!(
        diagnostics.iter().all(|d| d.session_started_at == session),
        "diagnostics must share the same session anchor as ledger"
    );

    for d in &diagnostics {
        assert_eq!(
            d.elapsed_since_session(),
            elapsed_since_session(d.recorded_at, d.session_started_at),
        );
    }

    for row in &ledger_rows {
        assert_eq!(
            elapsed_since_session(row.recorded_at, row.session_started_at),
            row.recorded_at.saturating_duration_since(session),
        );
    }
}

#[tokio::test]
async fn given_twin_boot_when_observing_streams_then_recorded_at_monotonic_within_each_stream() {
    let (diag_tx, mut diag_rx) = mpsc::unbounded_channel();
    let (trans_tx, mut trans_rx) = mpsc::channel(16);

    let runtime_options = VehicleControllerRuntimeOptions {
        diagnostic_tx: Some(diag_tx),
        transition_tx: Some(trans_tx),
        ..VehicleControllerRuntimeOptions::default()
    };

    let (controller, handle) = VehicleController::install_and_start_with_options(
        "OBS-STREAMS-02".to_string(),
        runtime_options,
    )
    .await
    .expect("start actor");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    power_on_to_idle(&controller).await;

    let mut ledger_rows = vec![];
    while let Ok(row) = trans_rx.try_recv() {
        ledger_rows.push(row);
    }
    let diagnostics = drain_diagnostics(&mut diag_rx, Duration::from_millis(100)).await;

    for pair in ledger_rows.windows(2) {
        assert!(
            pair[1].recorded_at >= pair[0].recorded_at,
            "ledger recorded_at must be non-decreasing"
        );
    }
    for pair in diagnostics.windows(2) {
        assert!(
            pair[1].recorded_at >= pair[0].recorded_at,
            "diagnostic recorded_at must be non-decreasing"
        );
    }
}

#[tokio::test]
async fn given_twin_with_transition_sink_only_when_boot_then_ledger_carries_session_pair() {
    let (trans_tx, mut trans_rx) = mpsc::channel(8);
    let runtime_options = VehicleControllerRuntimeOptions {
        transition_tx: Some(trans_tx),
        ..VehicleControllerRuntimeOptions::default()
    };

    let (controller, handle) = VehicleController::install_and_start_with_options(
        "OBS-STREAMS-03".to_string(),
        runtime_options,
    )
    .await
    .expect("start actor");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    power_on_to_idle(&controller).await;

    let row = trans_rx.recv().await.expect("first ledger row");
    assert!(row.session_started_at.unix_seconds() > 0 || row.session_started_at.nanosecond() > 0);
    assert!(row.recorded_at >= row.session_started_at);
}
