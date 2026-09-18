//! Pure FLCM status and diagnostic contracts.

use std::time::Duration;

use common::digital_twin::ZoneSpontaneousEvent;
use common::fsm::{AssemblyId, FsmState};
use common::observation_records::diagnostic::{DiagnosticKind, DiagnosticLevel, DiagnosticRecord};
use common::observation_records::transition::SessionClock;
use common::twin_runtime::FLCM_SILENCE_THRESHOLD;
use common::vehicle_state::{
    FlcmContext, FlcmMessage, FlcmZoneReply, ObservationDisposition, ObservedBool,
};
use common::{
    ObservedEcuSignal, TwinIngressEvent, TwinMessage, VehicleController,
    VehicleControllerRuntimeOptions,
};

#[test]
fn flcm_defaults_to_unknown_and_not_silent() {
    let ctx = FlcmContext::default();

    assert_eq!(ctx.left_low_beam_status, ObservedBool::Unknown);
    assert_eq!(ctx.right_low_beam_status, ObservedBool::Unknown);
    assert!(!ctx.silent);
}

#[test]
fn flcm_ok_observations_set_each_side_on_independently() {
    let left =
        FlcmContext::default().on_receiving_message(FlcmMessage::LeftLowBeamStatusObserved(true));
    assert_eq!(left.disposition, ObservationDisposition::Initial);
    assert_eq!(left.ctx.left_low_beam_status, ObservedBool::On);
    assert_eq!(left.ctx.right_low_beam_status, ObservedBool::Unknown);

    let both = left
        .ctx
        .on_receiving_message(FlcmMessage::RightLowBeamStatusObserved(true));
    assert_eq!(both.ctx.left_low_beam_status, ObservedBool::On);
    assert_eq!(both.ctx.right_low_beam_status, ObservedBool::On);
}

#[test]
fn flcm_fail_observation_sets_status_off() {
    let reply =
        FlcmContext::default().on_receiving_message(FlcmMessage::LeftLowBeamStatusObserved(false));

    assert_eq!(reply.ctx.left_low_beam_status, ObservedBool::Off);
    assert!(reply.ctx.has_fault());
}

#[test]
fn flcm_is_confirmed_healthy_only_when_both_sides_are_ok_and_not_silent() {
    let left_only = FlcmContext::default()
        .on_receiving_message(FlcmMessage::LeftLowBeamStatusObserved(true))
        .ctx;
    assert!(!left_only.is_confirmed_healthy());

    let both = left_only
        .on_receiving_message(FlcmMessage::RightLowBeamStatusObserved(true))
        .ctx;
    assert!(both.is_confirmed_healthy());
    assert!(!both.with_silent(true).is_confirmed_healthy());
}

#[test]
fn flcm_silence_helper_marks_fault_without_changing_statuses() {
    let healthy = FlcmContext::default()
        .on_receiving_message(FlcmMessage::LeftLowBeamStatusObserved(true))
        .ctx
        .on_receiving_message(FlcmMessage::RightLowBeamStatusObserved(true))
        .ctx;
    let silent = healthy.with_silent(true);

    assert_eq!(silent.left_low_beam_status, ObservedBool::On);
    assert_eq!(silent.right_low_beam_status, ObservedBool::On);
    assert!(silent.silent);
    assert!(silent.has_fault());
}

#[test]
fn flcm_become_on_and_off_reset_observation_state() {
    let faulted = FlcmContext::default()
        .on_receiving_message(FlcmMessage::LeftLowBeamStatusObserved(false))
        .ctx
        .with_silent(true);

    for message in [FlcmMessage::BecomeOn, FlcmMessage::BecomeOff] {
        let reset = faulted.on_receiving_message(message);
        assert_eq!(reset.disposition, ObservationDisposition::Lifecycle);
        assert_eq!(reset.ctx, FlcmContext::default());
    }
}

#[test]
fn flcm_diagnostic_format_includes_silent_and_fail_facts() {
    let record = DiagnosticRecord::warning(
        &SessionClock::capture(),
        "VirtualCarActor",
        DiagnosticKind::FlcmLampFault {
            silent: true,
            left_fail: true,
            right_fail: false,
        },
    );

    let rendered = record.to_string().to_ascii_lowercase();
    assert!(rendered.contains("flcm"));
    assert!(rendered.contains("silent=true"));
    assert!(rendered.contains("left_fail=true"));
    assert!(rendered.contains("right_fail=false"));
    assert!(rendered.is_ascii());
}

async fn wait_for_idle(controller: &VehicleController) {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        if controller
            .get_snapshot(Some(Duration::from_millis(50)))
            .await
            .is_ok_and(|snapshot| *snapshot.current_state() == FsmState::Idle)
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for Idle"
        );
        tokio::task::yield_now().await;
    }
}

async fn wait_for_flcm(controller: &VehicleController, expected: FlcmContext) {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        if controller
            .get_snapshot(Some(Duration::from_millis(50)))
            .await
            .is_ok_and(|snapshot| snapshot.context().flcm == expected)
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for FLCM context {expected:?}"
        );
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn runtime_projects_flcm_status_into_twin_context() {
    let (controller, handle) = VehicleController::install_and_start("FLCM-PROJECTION".into())
        .await
        .expect("install controller");
    controller.send_power_on().await.expect("power on");
    wait_for_idle(&controller).await;

    controller
        .submit_twin_ingress(TwinIngressEvent::ObservedEcu(
            ObservedEcuSignal::LeftLowBeamStatus(true),
        ))
        .await
        .expect("left status");
    controller
        .submit_twin_ingress(TwinIngressEvent::ObservedEcu(
            ObservedEcuSignal::RightLowBeamStatus(false),
        ))
        .await
        .expect("right status");

    wait_for_flcm(
        &controller,
        FlcmContext {
            left_low_beam_status: ObservedBool::On,
            right_low_beam_status: ObservedBool::Off,
            silent: false,
        },
    )
    .await;
    controller.get_actor_ref().stop(None);
    handle.await.expect("controller task");
}

#[tokio::test]
async fn runtime_accepts_same_initial_status_after_power_cycle() {
    let (controller, handle) = VehicleController::install_and_start("FLCM-RESTART".into())
        .await
        .expect("install controller");
    controller.send_power_on().await.expect("first power on");
    wait_for_idle(&controller).await;
    controller
        .submit_twin_ingress(TwinIngressEvent::ObservedEcu(
            ObservedEcuSignal::LeftLowBeamStatus(true),
        ))
        .await
        .expect("first status");
    wait_for_flcm(
        &controller,
        FlcmContext {
            left_low_beam_status: ObservedBool::On,
            ..Default::default()
        },
    )
    .await;

    controller.send_power_off().await.expect("power off");
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        if controller
            .get_snapshot(Some(Duration::from_millis(50)))
            .await
            .is_ok_and(|snapshot| *snapshot.current_state() == FsmState::Off)
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "power-off timeout");
        tokio::task::yield_now().await;
    }
    controller.send_power_on().await.expect("second power on");
    wait_for_idle(&controller).await;
    controller
        .submit_twin_ingress(TwinIngressEvent::ObservedEcu(
            ObservedEcuSignal::LeftLowBeamStatus(true),
        ))
        .await
        .expect("status after restart");
    wait_for_flcm(
        &controller,
        FlcmContext {
            left_low_beam_status: ObservedBool::On,
            ..Default::default()
        },
    )
    .await;

    controller.get_actor_ref().stop(None);
    handle.await.expect("controller task");
}

#[tokio::test]
async fn runtime_warns_once_after_500ms_silence_and_clears_on_healthy_traffic() {
    let (diagnostic_tx, mut diagnostic_rx) = tokio::sync::mpsc::unbounded_channel();
    let options = VehicleControllerRuntimeOptions {
        diagnostic_tx: Some(diagnostic_tx),
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("FLCM-LIVENESS".into(), options)
            .await
            .expect("install controller");
    let boot = diagnostic_rx.recv().await.expect("boot diagnostic");
    assert_eq!(boot.kind, DiagnosticKind::Boot);

    controller.send_power_on().await.expect("power on");
    wait_for_idle(&controller).await;
    for signal in [
        ObservedEcuSignal::LeftLowBeamStatus(true),
        ObservedEcuSignal::RightLowBeamStatus(true),
    ] {
        controller
            .submit_twin_ingress(TwinIngressEvent::ObservedEcu(signal))
            .await
            .expect("healthy FLCM status");
    }
    let warning = tokio::time::timeout(Duration::from_millis(750), diagnostic_rx.recv())
        .await
        .expect("autonomous silence warning timeout")
        .expect("diagnostic channel closed");
    assert_eq!(warning.level, DiagnosticLevel::Warning);
    assert!(matches!(
        warning.kind,
        DiagnosticKind::FlcmLampFault {
            silent: true,
            left_fail: false,
            right_fail: false
        }
    ));

    assert!(
        tokio::time::timeout(Duration::from_millis(100), diagnostic_rx.recv())
            .await
            .is_err(),
        "silent interval emitted more than one warning"
    );

    controller
        .submit_twin_ingress(TwinIngressEvent::ObservedEcu(
            ObservedEcuSignal::LeftLowBeamStatus(true),
        ))
        .await
        .expect("resumed traffic");
    let cleared = tokio::time::timeout(Duration::from_millis(250), diagnostic_rx.recv())
        .await
        .expect("clear timeout")
        .expect("diagnostic channel closed");
    assert_eq!(cleared.level, DiagnosticLevel::Info);
    assert!(matches!(
        cleared.kind,
        DiagnosticKind::FlcmLampFault {
            silent: false,
            left_fail: false,
            right_fail: false
        }
    ));

    controller.get_actor_ref().stop(None);
    handle.await.expect("controller task");
}

/// A topology that never publishes FLCM status (ObservedEcus demos, emulator-only runs)
/// must stay `Unknown` — the watchdog is armed by the first observation, not by power-on.
#[tokio::test]
async fn runtime_never_warns_when_flcm_was_never_observed() {
    let (diagnostic_tx, mut diagnostic_rx) = tokio::sync::mpsc::unbounded_channel();
    let options = VehicleControllerRuntimeOptions {
        diagnostic_tx: Some(diagnostic_tx),
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("FLCM-NEVER-OBSERVED".into(), options)
            .await
            .expect("install controller");
    assert_eq!(
        diagnostic_rx.recv().await.expect("boot diagnostic").kind,
        DiagnosticKind::Boot
    );

    controller.send_power_on().await.expect("power on");
    wait_for_idle(&controller).await;

    tokio::time::sleep(FLCM_SILENCE_THRESHOLD + Duration::from_millis(200)).await;

    assert!(
        tokio::time::timeout(Duration::from_millis(50), diagnostic_rx.recv())
            .await
            .is_err(),
        "power-on without any FLCM traffic must not emit FlcmLampFault"
    );
    let snapshot = controller
        .get_snapshot(Some(Duration::from_millis(250)))
        .await
        .expect("snapshot after quiet interval");
    assert_eq!(snapshot.context().flcm, FlcmContext::default());

    controller.get_actor_ref().stop(None);
    handle.await.expect("controller task");
}

/// Brain ticks are a passthrough turn; the actor-owned deadline is the only silence source.
#[tokio::test]
async fn runtime_timer_ticks_do_not_decide_flcm_silence() {
    let (diagnostic_tx, mut diagnostic_rx) = tokio::sync::mpsc::unbounded_channel();
    let options = VehicleControllerRuntimeOptions {
        diagnostic_tx: Some(diagnostic_tx),
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("FLCM-TICK-PASSTHROUGH".into(), options)
            .await
            .expect("install controller");
    assert_eq!(
        diagnostic_rx.recv().await.expect("boot diagnostic").kind,
        DiagnosticKind::Boot
    );

    controller.send_power_on().await.expect("power on");
    wait_for_idle(&controller).await;

    for _ in 0..3 {
        controller
            .submit_twin_ingress(TwinIngressEvent::TimerTick)
            .await
            .expect("timer tick");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let snapshot = controller
        .get_snapshot(Some(Duration::from_millis(250)))
        .await
        .expect("snapshot after ticks");
    assert!(
        !snapshot.context().flcm.silent,
        "TimerTick must not decide FLCM silence"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), diagnostic_rx.recv())
            .await
            .is_err(),
        "TimerTick must not emit FlcmLampFault"
    );

    controller.get_actor_ref().stop(None);
    handle.await.expect("controller task");
}

#[tokio::test]
async fn runtime_ignores_queued_flcm_silence_completion_after_power_off() {
    let (diagnostic_tx, mut diagnostic_rx) = tokio::sync::mpsc::unbounded_channel();
    let options = VehicleControllerRuntimeOptions {
        diagnostic_tx: Some(diagnostic_tx),
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("FLCM-POWER-OFF-RACE".into(), options)
            .await
            .expect("install controller");
    assert_eq!(
        diagnostic_rx.recv().await.expect("boot diagnostic").kind,
        DiagnosticKind::Boot
    );

    controller.send_power_on().await.expect("power on");
    wait_for_idle(&controller).await;
    controller.send_power_off().await.expect("power off");
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        if controller
            .get_snapshot(Some(Duration::from_millis(50)))
            .await
            .is_ok_and(|snapshot| *snapshot.current_state() == FsmState::Off)
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "power-off timeout");
        tokio::task::yield_now().await;
    }

    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneSpontaneous {
            zone_id: AssemblyId::Flcm,
            event: ZoneSpontaneousEvent::Flcm {
                reply: FlcmZoneReply {
                    ctx: FlcmContext {
                        silent: true,
                        ..Default::default()
                    },
                    disposition: ObservationDisposition::Lifecycle,
                },
            },
        })
        .expect("inject queued silence completion");

    tokio::time::sleep(Duration::from_millis(25)).await;
    let snapshot = controller
        .get_snapshot(Some(Duration::from_millis(250)))
        .await
        .expect("snapshot after queued completion");
    assert_eq!(*snapshot.current_state(), FsmState::Off);
    assert_eq!(snapshot.context().flcm, FlcmContext::default());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), diagnostic_rx.recv())
            .await
            .is_err(),
        "queued silence completion emitted a diagnostic while unpowered"
    );

    controller.get_actor_ref().stop(None);
    handle.await.expect("controller task");
}
