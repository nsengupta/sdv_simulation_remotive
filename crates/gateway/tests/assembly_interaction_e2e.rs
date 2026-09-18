//! Cross-crate assembly interaction: broker-format CAN through Gateway projection,
//! Twin children, deterministic ledger, and TUI-facing published DTOs.
//!
//! Scope:
//! - Encodes the same internal carriers the Remotive bridge writes
//!   (`0x105`/`0x106`/`0x107` and FLCM status `0x108`/`0x109`).
//! - Projects those frames through the public Gateway mapping into `TwinIngressEvent`.
//! - Drives the public Twin ingress seam (`submit_twin_ingress`).
//! - Proves SCCM/BCM observation, ingress-ordered ledger, duplicate suppression,
//!   empty actions (no Twin `SetTurnLights`), and controlled PowerOff.
//! - Proves FLCM Ok/Fail status into ledger/context, Warning diagnostics, and
//!   autonomous 500 ms silence.
//!
//! Non-scope:
//! - Live Remotive broker / pytest (operator live acceptance).
//! - SocketCAN `vcan0` transport and standalone actuator processes.

use std::time::Duration;

use common::facade::{
    AssemblyTopology, BcmState, DiagnosticKind, DiagnosticLevel, DiagnosticRecord,
    LifecycleCommand, ObservedEcuSignal, PublishedBcmState, PublishedDomainAction,
    PublishedFsmEvent, PublishedFsmState, PublishedObservedBool, PublishedTransitionRecord,
    TwinIngressEvent, VehicleController, VehicleControllerRuntimeOptions,
};
use common::fsm::FsmState;
use common::vehicle_state::ObservedBool;
use gateway::ingress::can_frame_to_twin_ingress;
use socketcan::Frame;
use tokio::sync::mpsc;

const IDENTITY: &str = "E2E-ASSEMBLY-01";

async fn submit_ingress(controller: &VehicleController, event: TwinIngressEvent) {
    controller
        .submit_twin_ingress(event)
        .await
        .expect("public Twin ingress");
}

fn project_bridge_can(signal: ObservedEcuSignal) -> TwinIngressEvent {
    let frame = signal.to_can_frame().expect("encode bridge-format CAN");
    match signal {
        ObservedEcuSignal::HazardButton(_) => assert_eq!(frame.raw_id(), 0x105),
        ObservedEcuSignal::LeftTurnRequest(_) => assert_eq!(frame.raw_id(), 0x106),
        ObservedEcuSignal::RightTurnRequest(_) => assert_eq!(frame.raw_id(), 0x107),
        ObservedEcuSignal::LeftLowBeamStatus(_) => assert_eq!(frame.raw_id(), 0x108),
        ObservedEcuSignal::RightLowBeamStatus(_) => assert_eq!(frame.raw_id(), 0x109),
    }
    can_frame_to_twin_ingress(&frame).unwrap_or_else(|| {
        panic!("Gateway must project {signal:?} on {frame:?}");
    })
}

async fn submit_observed(controller: &VehicleController, signal: ObservedEcuSignal) {
    submit_ingress(controller, project_bridge_can(signal)).await;
}

async fn wait_fsm_state(controller: &VehicleController, expected: FsmState, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(snapshot) = controller
            .get_snapshot(Some(Duration::from_millis(50)))
            .await
            && *snapshot.current_state() == expected
        {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("timed out after {timeout:?} waiting for FSM {expected:?}");
        }
        tokio::task::yield_now().await;
    }
}

async fn recv_row(rx: &mut mpsc::Receiver<PublishedTransitionRecord>) -> PublishedTransitionRecord {
    tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .expect("transition row timeout")
        .expect("transition channel closed")
}

async fn assert_no_extra_rows(rx: &mut mpsc::Receiver<PublishedTransitionRecord>) {
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(
        rx.try_recv().is_err(),
        "duplicate or extra observation must not emit a ledger row"
    );
}

fn assert_no_twin_actuation(row: &PublishedTransitionRecord) {
    assert!(
        !row.actions
            .iter()
            .any(|action| matches!(action, PublishedDomainAction::SetTurnLights { .. })),
        "Twin must not emit SetTurnLights; got {:?}",
        row.actions
    );
}

fn assert_observation_actions_empty(row: &PublishedTransitionRecord) {
    assert_no_twin_actuation(row);
    assert!(
        row.actions.is_empty(),
        "observation rows must emit empty actions; got {:?}",
        row.actions
    );
}

fn tui_observed_label(value: PublishedObservedBool) -> &'static str {
    match value {
        PublishedObservedBool::Unknown => "UNKNOWN",
        PublishedObservedBool::Off => "OFF",
        PublishedObservedBool::On => "ON",
    }
}

fn assert_flcm_ctx(
    row: &PublishedTransitionRecord,
    left: PublishedObservedBool,
    right: PublishedObservedBool,
    silent: bool,
) {
    assert_eq!(row.current_ctx.flcm.left_low_beam_status_ok, left);
    assert_eq!(row.current_ctx.flcm.right_low_beam_status_ok, right);
    assert_eq!(row.current_ctx.flcm.silent, silent);
}

async fn recv_diagnostic(rx: &mut mpsc::UnboundedReceiver<DiagnosticRecord>) -> DiagnosticRecord {
    tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .expect("diagnostic timeout")
        .expect("diagnostic channel closed")
}

fn assert_flcm_fault(
    record: &DiagnosticRecord,
    level: DiagnosticLevel,
    silent: bool,
    left_fail: bool,
    right_fail: bool,
) {
    assert_eq!(record.level, level);
    assert_eq!(
        record.kind,
        DiagnosticKind::FlcmLampFault {
            silent,
            left_fail,
            right_fail,
        }
    );
}

async fn power_on_and_drain_startup(
    controller: &VehicleController,
    transition_rx: &mut mpsc::Receiver<PublishedTransitionRecord>,
) {
    let power_on = LifecycleCommand::PowerOn
        .to_can_frame()
        .expect("encode PowerOn");
    submit_ingress(
        controller,
        can_frame_to_twin_ingress(&power_on).expect("Gateway maps PowerOn"),
    )
    .await;
    wait_fsm_state(controller, FsmState::Idle, Duration::from_millis(500)).await;

    let power_on_row = recv_row(transition_rx).await;
    assert_eq!(power_on_row.event, PublishedFsmEvent::PowerOn);
    let _sccm_ready = recv_row(transition_rx).await;
    let bcm_ready = recv_row(transition_rx).await;
    assert_eq!(bcm_ready.next_state, PublishedFsmState::Idle);
    assert_eq!(bcm_ready.current_ctx.bcm.state, PublishedBcmState::Ready);
    assert_flcm_ctx(
        &bcm_ready,
        PublishedObservedBool::Unknown,
        PublishedObservedBool::Unknown,
        false,
    );
}

fn assert_tui_dtos(
    row: &PublishedTransitionRecord,
    hazard: &'static str,
    left: &'static str,
    right: &'static str,
    bcm: PublishedBcmState,
) {
    assert_eq!(
        tui_observed_label(row.current_ctx.sccm.hazard_mode_on),
        hazard
    );
    assert_eq!(
        tui_observed_label(row.current_ctx.bcm.left_turn_request_on),
        left
    );
    assert_eq!(
        tui_observed_label(row.current_ctx.bcm.right_turn_request_on),
        right
    );
    assert_eq!(row.current_ctx.bcm.state, bcm);
}

#[tokio::test]
async fn observed_hazard_left_and_right_cross_gateway_twin_ledger_and_tui_dtos() {
    let (transition_tx, mut transition_rx) = mpsc::channel(32);
    let (actuation_tx, mut actuation_rx) = mpsc::channel(16);
    let runtime_options = VehicleControllerRuntimeOptions {
        assembly_topology: AssemblyTopology::ObservedEcus,
        transition_tx: Some(transition_tx),
        actuation_command_tx: Some(actuation_tx),
        ..Default::default()
    };
    let (controller, _join) =
        VehicleController::install_and_start_with_options(IDENTITY.to_string(), runtime_options)
            .await
            .expect("controller start");

    let power_on = LifecycleCommand::PowerOn
        .to_can_frame()
        .expect("encode PowerOn");
    submit_ingress(
        &controller,
        can_frame_to_twin_ingress(&power_on).expect("Gateway maps PowerOn"),
    )
    .await;
    wait_fsm_state(&controller, FsmState::Idle, Duration::from_millis(500)).await;

    let power_on_row = recv_row(&mut transition_rx).await;
    assert_eq!(power_on_row.event, PublishedFsmEvent::PowerOn);
    let sccm_ready = recv_row(&mut transition_rx).await;
    let bcm_ready = recv_row(&mut transition_rx).await;
    assert_eq!(sccm_ready.event, PublishedFsmEvent::TimerTick);
    assert_eq!(bcm_ready.event, PublishedFsmEvent::TimerTick);
    assert_eq!(bcm_ready.next_state, PublishedFsmState::Idle);
    assert_eq!(bcm_ready.current_ctx.bcm.state, PublishedBcmState::Ready);
    assert_no_twin_actuation(&power_on_row);
    assert_no_twin_actuation(&sccm_ready);
    assert_no_twin_actuation(&bcm_ready);
    assert_tui_dtos(
        &bcm_ready,
        "OFF",
        "UNKNOWN",
        "UNKNOWN",
        PublishedBcmState::Ready,
    );

    let ready = controller
        .get_snapshot(Some(Duration::from_millis(300)))
        .await
        .expect("ready snapshot");
    assert_eq!(*ready.current_state(), FsmState::Idle);
    assert_eq!(ready.context().bcm.state, BcmState::Ready);
    assert_eq!(ready.context().sccm.hazard_button, ObservedBool::Unknown);
    assert_eq!(ready.context().bcm.left_turn_request, ObservedBool::Unknown);
    assert_eq!(
        ready.context().bcm.right_turn_request,
        ObservedBool::Unknown
    );

    submit_observed(&controller, ObservedEcuSignal::HazardButton(false)).await;
    let hazard_off = recv_row(&mut transition_rx).await;
    assert_eq!(
        hazard_off.event,
        PublishedFsmEvent::HazardButtonObserved(false)
    );
    assert_eq!(
        hazard_off.current_ctx.sccm.hazard_button_on,
        PublishedObservedBool::Off
    );
    assert_eq!(
        hazard_off.current_ctx.sccm.hazard_mode_on,
        PublishedObservedBool::Off
    );
    assert_eq!(
        hazard_off.current_ctx.bcm.left_turn_request_on,
        PublishedObservedBool::Unknown
    );
    assert_eq!(
        hazard_off.current_ctx.bcm.right_turn_request_on,
        PublishedObservedBool::Unknown
    );
    assert_observation_actions_empty(&hazard_off);
    assert_tui_dtos(
        &hazard_off,
        "OFF",
        "UNKNOWN",
        "UNKNOWN",
        PublishedBcmState::Ready,
    );

    submit_observed(&controller, ObservedEcuSignal::HazardButton(false)).await;
    submit_observed(&controller, ObservedEcuSignal::HazardButton(false)).await;
    assert_no_extra_rows(&mut transition_rx).await;

    submit_observed(&controller, ObservedEcuSignal::HazardButton(true)).await;
    let hazard_on = recv_row(&mut transition_rx).await;
    assert_eq!(
        hazard_on.event,
        PublishedFsmEvent::HazardButtonObserved(true)
    );
    assert_eq!(
        hazard_on.current_ctx.sccm.hazard_button_on,
        PublishedObservedBool::On
    );
    assert_eq!(
        hazard_on.current_ctx.sccm.hazard_mode_on,
        PublishedObservedBool::On
    );
    assert_eq!(
        hazard_on.current_ctx.bcm.left_turn_request_on,
        PublishedObservedBool::Unknown
    );
    assert_observation_actions_empty(&hazard_on);

    submit_observed(&controller, ObservedEcuSignal::HazardButton(false)).await;
    let hazard_wire_off = recv_row(&mut transition_rx).await;
    assert_eq!(
        hazard_wire_off.event,
        PublishedFsmEvent::HazardButtonObserved(false)
    );
    assert_eq!(
        hazard_wire_off.current_ctx.sccm.hazard_button_on,
        PublishedObservedBool::Off
    );
    assert_eq!(
        hazard_wire_off.current_ctx.sccm.hazard_mode_on,
        PublishedObservedBool::On
    );
    assert_observation_actions_empty(&hazard_wire_off);
    assert_tui_dtos(
        &hazard_wire_off,
        "ON",
        "UNKNOWN",
        "UNKNOWN",
        PublishedBcmState::Ready,
    );

    let after_pulse = controller
        .get_snapshot(Some(Duration::from_millis(300)))
        .await
        .expect("hazard pulse-off snapshot");
    assert_eq!(after_pulse.context().sccm.hazard_button, ObservedBool::Off);
    assert_eq!(after_pulse.context().sccm.hazard_mode, ObservedBool::On);

    submit_observed(&controller, ObservedEcuSignal::HazardButton(true)).await;
    let hazard_second_on = recv_row(&mut transition_rx).await;
    assert_eq!(
        hazard_second_on.event,
        PublishedFsmEvent::HazardButtonObserved(true)
    );
    assert_eq!(
        hazard_second_on.current_ctx.sccm.hazard_button_on,
        PublishedObservedBool::On
    );
    assert_eq!(
        hazard_second_on.current_ctx.sccm.hazard_mode_on,
        PublishedObservedBool::Off
    );
    assert_observation_actions_empty(&hazard_second_on);
    assert_tui_dtos(
        &hazard_second_on,
        "OFF",
        "UNKNOWN",
        "UNKNOWN",
        PublishedBcmState::Ready,
    );

    let after_second = controller
        .get_snapshot(Some(Duration::from_millis(300)))
        .await
        .expect("second hazard edge snapshot");
    assert_eq!(after_second.context().sccm.hazard_button, ObservedBool::On);
    assert_eq!(after_second.context().sccm.hazard_mode, ObservedBool::Off);

    submit_observed(&controller, ObservedEcuSignal::LeftTurnRequest(false)).await;
    let left_off = recv_row(&mut transition_rx).await;
    assert_eq!(
        left_off.event,
        PublishedFsmEvent::LeftTurnRequestObserved(false)
    );
    assert_eq!(
        left_off.current_ctx.bcm.left_turn_request_on,
        PublishedObservedBool::Off
    );
    assert_eq!(
        left_off.current_ctx.bcm.right_turn_request_on,
        PublishedObservedBool::Unknown
    );
    assert_eq!(
        left_off.current_ctx.sccm.hazard_button_on,
        PublishedObservedBool::On
    );
    assert_observation_actions_empty(&left_off);

    submit_observed(&controller, ObservedEcuSignal::LeftTurnRequest(false)).await;
    assert_no_extra_rows(&mut transition_rx).await;

    submit_observed(&controller, ObservedEcuSignal::RightTurnRequest(false)).await;
    let right_off = recv_row(&mut transition_rx).await;
    assert_eq!(
        right_off.event,
        PublishedFsmEvent::RightTurnRequestObserved(false)
    );
    assert_eq!(
        right_off.current_ctx.bcm.right_turn_request_on,
        PublishedObservedBool::Off
    );
    assert_eq!(
        right_off.current_ctx.bcm.left_turn_request_on,
        PublishedObservedBool::Off
    );
    assert_observation_actions_empty(&right_off);

    submit_observed(&controller, ObservedEcuSignal::RightTurnRequest(false)).await;
    assert_no_extra_rows(&mut transition_rx).await;

    submit_observed(&controller, ObservedEcuSignal::LeftTurnRequest(true)).await;
    let left_on = recv_row(&mut transition_rx).await;
    assert_eq!(
        left_on.event,
        PublishedFsmEvent::LeftTurnRequestObserved(true)
    );
    assert_eq!(
        left_on.current_ctx.bcm.left_turn_request_on,
        PublishedObservedBool::On
    );
    assert_eq!(
        left_on.current_ctx.bcm.right_turn_request_on,
        PublishedObservedBool::Off
    );
    assert_observation_actions_empty(&left_on);

    submit_observed(&controller, ObservedEcuSignal::RightTurnRequest(true)).await;
    let right_on = recv_row(&mut transition_rx).await;
    assert_eq!(
        right_on.event,
        PublishedFsmEvent::RightTurnRequestObserved(true)
    );
    assert_eq!(
        right_on.current_ctx.bcm.right_turn_request_on,
        PublishedObservedBool::On
    );
    assert_eq!(
        right_on.current_ctx.bcm.left_turn_request_on,
        PublishedObservedBool::On
    );
    assert_eq!(
        right_on.current_ctx.sccm.hazard_button_on,
        PublishedObservedBool::On
    );
    assert_observation_actions_empty(&right_on);
    assert_tui_dtos(&right_on, "OFF", "ON", "ON", PublishedBcmState::Ready);

    submit_observed(&controller, ObservedEcuSignal::HazardButton(true)).await;
    submit_observed(&controller, ObservedEcuSignal::LeftTurnRequest(true)).await;
    submit_observed(&controller, ObservedEcuSignal::RightTurnRequest(true)).await;
    assert_no_extra_rows(&mut transition_rx).await;

    let observation_rows = [
        &hazard_off,
        &hazard_on,
        &hazard_wire_off,
        &hazard_second_on,
        &left_off,
        &right_off,
        &left_on,
        &right_on,
    ];
    assert!(
        observation_rows
            .windows(2)
            .all(|pair| pair[1].record_seq == pair[0].record_seq + 1),
        "ledger order must follow ingress order with no duplicate holes"
    );

    let snapshot = controller
        .get_snapshot(Some(Duration::from_millis(300)))
        .await
        .expect("observed snapshot");
    assert_eq!(snapshot.context().sccm.hazard_button, ObservedBool::On);
    assert_eq!(snapshot.context().sccm.hazard_mode, ObservedBool::Off);
    assert_eq!(snapshot.context().bcm.left_turn_request, ObservedBool::On);
    assert_eq!(snapshot.context().bcm.right_turn_request, ObservedBool::On);
    assert_eq!(snapshot.context().bcm.state, BcmState::Ready);
    assert_eq!(*snapshot.current_state(), FsmState::Idle);

    assert!(
        actuation_rx.try_recv().is_err(),
        "Twin must not enqueue actuation commands"
    );

    let power_off = LifecycleCommand::PowerOff
        .to_can_frame()
        .expect("encode PowerOff");
    submit_ingress(
        &controller,
        can_frame_to_twin_ingress(&power_off).expect("Gateway maps PowerOff"),
    )
    .await;
    wait_fsm_state(&controller, FsmState::Off, Duration::from_millis(500)).await;

    let mut saw_power_off = false;
    let mut last = None;
    while let Ok(row) = tokio::time::timeout(Duration::from_millis(200), transition_rx.recv()).await
    {
        let Some(row) = row else {
            break;
        };
        assert_no_twin_actuation(&row);
        if row.event == PublishedFsmEvent::PowerOff {
            saw_power_off = true;
        }
        last = Some(row);
    }
    assert!(
        saw_power_off,
        "controlled PowerOff must appear in the ledger"
    );
    let last = last.expect("shutdown must publish at least one row");
    assert_eq!(last.next_state, PublishedFsmState::Off);
    assert_eq!(
        last.current_ctx.sccm.hazard_mode_on,
        PublishedObservedBool::Off
    );
    assert_tui_dtos(&last, "OFF", "ON", "ON", PublishedBcmState::Off);

    let off = controller
        .get_snapshot(Some(Duration::from_millis(300)))
        .await
        .expect("off snapshot");
    assert_eq!(*off.current_state(), FsmState::Off);
    assert_eq!(off.context().bcm.state, BcmState::Off);
    assert_eq!(off.context().sccm.hazard_button, ObservedBool::On);
    assert_eq!(off.context().sccm.hazard_mode, ObservedBool::Off);
    assert_eq!(off.context().bcm.left_turn_request, ObservedBool::On);
    assert_eq!(off.context().bcm.right_turn_request, ObservedBool::On);
    assert!(
        actuation_rx.try_recv().is_err(),
        "PowerOff must not invent Twin SetTurnLights actuation"
    );
}

#[tokio::test]
async fn observed_flcm_ok_and_fail_cross_gateway_twin_ledger_and_warning() {
    let (transition_tx, mut transition_rx) = mpsc::channel(32);
    let (diagnostic_tx, mut diagnostic_rx) = mpsc::unbounded_channel();
    let (actuation_tx, mut actuation_rx) = mpsc::channel(16);
    let runtime_options = VehicleControllerRuntimeOptions {
        assembly_topology: AssemblyTopology::ObservedEcus,
        transition_tx: Some(transition_tx),
        diagnostic_tx: Some(diagnostic_tx),
        actuation_command_tx: Some(actuation_tx),
        ..Default::default()
    };
    let (controller, _join) = VehicleController::install_and_start_with_options(
        "E2E-FLCM-STATUS-01".into(),
        runtime_options,
    )
    .await
    .expect("controller start");

    let boot = recv_diagnostic(&mut diagnostic_rx).await;
    assert_eq!(boot.kind, DiagnosticKind::Boot);

    power_on_and_drain_startup(&controller, &mut transition_rx).await;

    let ready = controller
        .get_snapshot(Some(Duration::from_millis(300)))
        .await
        .expect("ready snapshot");
    assert_eq!(
        ready.context().flcm.left_low_beam_status,
        ObservedBool::Unknown
    );
    assert_eq!(
        ready.context().flcm.right_low_beam_status,
        ObservedBool::Unknown
    );
    assert!(!ready.context().flcm.silent);

    submit_observed(&controller, ObservedEcuSignal::LeftLowBeamStatus(true)).await;
    let left_ok = recv_row(&mut transition_rx).await;
    assert_eq!(
        left_ok.event,
        PublishedFsmEvent::LeftLowBeamStatusObserved(true)
    );
    assert_flcm_ctx(
        &left_ok,
        PublishedObservedBool::On,
        PublishedObservedBool::Unknown,
        false,
    );
    assert_observation_actions_empty(&left_ok);

    submit_observed(&controller, ObservedEcuSignal::LeftLowBeamStatus(true)).await;
    assert_no_extra_rows(&mut transition_rx).await;

    submit_observed(&controller, ObservedEcuSignal::RightLowBeamStatus(true)).await;
    let right_ok = recv_row(&mut transition_rx).await;
    assert_eq!(
        right_ok.event,
        PublishedFsmEvent::RightLowBeamStatusObserved(true)
    );
    assert_flcm_ctx(
        &right_ok,
        PublishedObservedBool::On,
        PublishedObservedBool::On,
        false,
    );
    assert_observation_actions_empty(&right_ok);

    let healthy = controller
        .get_snapshot(Some(Duration::from_millis(300)))
        .await
        .expect("healthy FLCM snapshot");
    assert_eq!(
        healthy.context().flcm.left_low_beam_status,
        ObservedBool::On
    );
    assert_eq!(
        healthy.context().flcm.right_low_beam_status,
        ObservedBool::On
    );
    assert!(!healthy.context().flcm.silent);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), diagnostic_rx.recv())
            .await
            .is_err(),
        "Ok/Ok must not emit FlcmLampFault"
    );

    submit_observed(&controller, ObservedEcuSignal::LeftLowBeamStatus(false)).await;
    let left_fail = recv_row(&mut transition_rx).await;
    assert_eq!(
        left_fail.event,
        PublishedFsmEvent::LeftLowBeamStatusObserved(false)
    );
    assert_flcm_ctx(
        &left_fail,
        PublishedObservedBool::Off,
        PublishedObservedBool::On,
        false,
    );
    assert_observation_actions_empty(&left_fail);

    let fail_snapshot = controller
        .get_snapshot(Some(Duration::from_millis(300)))
        .await
        .expect("left Fail snapshot");
    assert_eq!(
        fail_snapshot.context().flcm.left_low_beam_status,
        ObservedBool::Off
    );
    assert_eq!(
        fail_snapshot.context().flcm.right_low_beam_status,
        ObservedBool::On
    );
    assert!(!fail_snapshot.context().flcm.silent);

    let warning = recv_diagnostic(&mut diagnostic_rx).await;
    assert_flcm_fault(&warning, DiagnosticLevel::Warning, false, true, false);

    submit_observed(&controller, ObservedEcuSignal::RightLowBeamStatus(false)).await;
    let right_fail = recv_row(&mut transition_rx).await;
    assert_flcm_ctx(
        &right_fail,
        PublishedObservedBool::Off,
        PublishedObservedBool::Off,
        false,
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(40), diagnostic_rx.recv())
            .await
            .is_err(),
        "latched Warning must not re-emit on the second Fail"
    );

    submit_observed(&controller, ObservedEcuSignal::LeftLowBeamStatus(true)).await;
    let left_resume = recv_row(&mut transition_rx).await;
    assert_flcm_ctx(
        &left_resume,
        PublishedObservedBool::On,
        PublishedObservedBool::Off,
        false,
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(40), diagnostic_rx.recv())
            .await
            .is_err(),
        "one remaining Fail is not confirmed-healthy; must not clear Warning"
    );

    submit_observed(&controller, ObservedEcuSignal::RightLowBeamStatus(true)).await;
    let both_ok = recv_row(&mut transition_rx).await;
    assert_flcm_ctx(
        &both_ok,
        PublishedObservedBool::On,
        PublishedObservedBool::On,
        false,
    );
    let cleared = recv_diagnostic(&mut diagnostic_rx).await;
    assert_flcm_fault(&cleared, DiagnosticLevel::Info, false, false, false);

    assert!(
        observation_rows_are_contiguous(&[
            &left_ok,
            &right_ok,
            &left_fail,
            &right_fail,
            &left_resume,
            &both_ok
        ]),
        "FLCM ledger order must follow ingress order with no duplicate holes"
    );
    assert!(
        actuation_rx.try_recv().is_err(),
        "FLCM status must not enqueue Twin actuation"
    );
}

fn observation_rows_are_contiguous(rows: &[&PublishedTransitionRecord]) -> bool {
    rows.windows(2)
        .all(|pair| pair[1].record_seq == pair[0].record_seq + 1)
}

#[tokio::test]
async fn observed_flcm_silence_sets_silent_flag_and_warning() {
    let (transition_tx, mut transition_rx) = mpsc::channel(32);
    let (diagnostic_tx, mut diagnostic_rx) = mpsc::unbounded_channel();
    let runtime_options = VehicleControllerRuntimeOptions {
        assembly_topology: AssemblyTopology::ObservedEcus,
        transition_tx: Some(transition_tx),
        diagnostic_tx: Some(diagnostic_tx),
        ..Default::default()
    };
    let (controller, _join) = VehicleController::install_and_start_with_options(
        "E2E-FLCM-SILENCE-01".into(),
        runtime_options,
    )
    .await
    .expect("controller start");

    let boot = recv_diagnostic(&mut diagnostic_rx).await;
    assert_eq!(boot.kind, DiagnosticKind::Boot);

    power_on_and_drain_startup(&controller, &mut transition_rx).await;

    submit_observed(&controller, ObservedEcuSignal::LeftLowBeamStatus(true)).await;
    let _left_ok = recv_row(&mut transition_rx).await;
    submit_observed(&controller, ObservedEcuSignal::RightLowBeamStatus(true)).await;
    let _right_ok = recv_row(&mut transition_rx).await;

    let silent_row = tokio::time::timeout(Duration::from_millis(750), transition_rx.recv())
        .await
        .expect("autonomous silence ledger timeout")
        .expect("transition channel closed");
    assert_eq!(silent_row.event, PublishedFsmEvent::TimerTick);
    assert_flcm_ctx(
        &silent_row,
        PublishedObservedBool::On,
        PublishedObservedBool::On,
        true,
    );
    assert_observation_actions_empty(&silent_row);

    let warning = tokio::time::timeout(Duration::from_millis(250), diagnostic_rx.recv())
        .await
        .expect("autonomous silence warning timeout")
        .expect("diagnostic channel closed");
    assert_flcm_fault(&warning, DiagnosticLevel::Warning, true, false, false);

    let snapshot = controller
        .get_snapshot(Some(Duration::from_millis(300)))
        .await
        .expect("silent snapshot");
    assert!(snapshot.context().flcm.silent);
    assert_eq!(
        snapshot.context().flcm.left_low_beam_status,
        ObservedBool::On
    );
    assert_eq!(
        snapshot.context().flcm.right_low_beam_status,
        ObservedBool::On
    );
}

/// Observed-ECU and emulator-only runs publish no FLCM status at all. Power-on
/// alone must not arm the watchdog, so those runs stay silent-free and ledger-quiet.
#[tokio::test]
async fn power_on_without_flcm_traffic_never_warns_or_writes_a_silent_row() {
    let (transition_tx, mut transition_rx) = mpsc::channel(32);
    let (diagnostic_tx, mut diagnostic_rx) = mpsc::unbounded_channel();
    let runtime_options = VehicleControllerRuntimeOptions {
        assembly_topology: AssemblyTopology::ObservedEcus,
        transition_tx: Some(transition_tx),
        diagnostic_tx: Some(diagnostic_tx),
        ..Default::default()
    };
    let (controller, _join) = VehicleController::install_and_start_with_options(
        "E2E-FLCM-NEVER-OBSERVED-01".into(),
        runtime_options,
    )
    .await
    .expect("controller start");

    let boot = recv_diagnostic(&mut diagnostic_rx).await;
    assert_eq!(boot.kind, DiagnosticKind::Boot);

    power_on_and_drain_startup(&controller, &mut transition_rx).await;

    tokio::time::sleep(Duration::from_millis(700)).await;

    assert!(
        transition_rx.try_recv().is_err(),
        "a run without FLCM traffic must not write a silence ledger row"
    );
    assert!(
        diagnostic_rx.try_recv().is_err(),
        "a run without FLCM traffic must not emit FlcmLampFault"
    );

    let snapshot = controller
        .get_snapshot(Some(Duration::from_millis(300)))
        .await
        .expect("quiet snapshot");
    assert!(!snapshot.context().flcm.silent);
    assert_eq!(
        snapshot.context().flcm.left_low_beam_status,
        ObservedBool::Unknown
    );
    assert_eq!(
        snapshot.context().flcm.right_low_beam_status,
        ObservedBool::Unknown
    );
}
