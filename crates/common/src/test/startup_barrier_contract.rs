//! Contract tests: `StartAssemblies` / `StopAssemblies` wired to real
//! `TurnBarrier` coordination (RED → GREEN in ).
//!
//! ## Older design (RED)
//!
//! An earlier cut left `StartAssemblies` / `StopAssemblies` as no-ops in
//! `apply_committed_quiescence`. The FSM enters `PreparingToStart` on `PowerOn`
//! but never transitions to `Idle` because no `AssemblyZoneReady` is committed.
//! `wait_fsm_state(Idle, 500ms)` times out → tests 1 and 2 fail.
//! Similarly for the shutdown path → tests 3 and 4 fail.
//!
//! ## Current design (GREEN)
//!
//! `StartAssemblies` creates a `TurnBarrier` per managed assembly, sends `BecomeOn`,
//! and the drain loop commits `AssemblyZoneReady(Headlamp)` when the headlamp replies.
//! The FSM transitions `PreparingToStart → Idle` (test 1) and stays in `PreparingToStart`
//! when no reply arrives (test 2). Likewise for shutdown (tests 3 and 4).

use std::time::Duration;

use tokio::sync::mpsc;

use crate::digital_twin::{TwinMessage, ZoneReply};
use crate::fsm::{AssemblyId, FsmEvent, FsmState, HeadlampState};
use crate::observation_records::{PublishedFsmEvent, PublishedFsmState};
use crate::test::{ActorGuard, power_on_to_idle, wait_fsm_state};
use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;
use crate::vehicle_state::{HeadlampContext, HeadlampZoneReply};
use crate::{TwinIngressEvent, VehicleController, VssSignal};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Turn ID allocated for the startup barrier (`StartAssemblies` loop, turn 2).
const STARTUP_BARRIER_TURN: u64 = 2;

/// Current startup allocates one barrier turn for each named assembly: headlamp and wiper.
/// These counts keep direct IDs confined to the silent-zone test seam without exposing
/// production turn-allocation internals.
const STARTUP_ASSEMBLY_BARRIER_COUNT: u64 = 2;
const QUEUED_POST_POWER_ON_INGRESS_COUNT: u64 = 4;

fn zone_reply_with_state(state: HeadlampState) -> ZoneReply {
    ZoneReply::Headlamp(HeadlampZoneReply {
        ctx: HeadlampContext {
            state,
            ack_pending_since: None,
        },
        outcomes: vec![],
    })
}

fn inject_zone_ready(controller: &VehicleController, turn_id: u64, state: HeadlampState) {
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Headlamp,
            turn_id,
            tell_attempt: 0,
            reply: zone_reply_with_state(state),
        })
        .expect("inject_zone_ready");
}

async fn spawn_non_silent(identity: &str) -> (VehicleController, ActorGuard<TwinMessage>) {
    let (controller, handle) =
        VehicleController::install_and_start_with_options(identity.to_string(), Default::default())
            .await
            .expect("spawn non-silent controller");
    let guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };
    (controller, guard)
}

async fn spawn_silent(identity: &str) -> (VehicleController, ActorGuard<TwinMessage>) {
    let opts = VehicleControllerRuntimeOptions {
        test_silent_headlamp: true,
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options(identity.to_string(), opts)
            .await
            .expect("spawn silent controller");
    let guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };
    (controller, guard)
}

// ── Test 1 ───────────────────────────────────────────────────────────────────

/// Non-silent headlamp replies to `BecomeOn`; brain commits `AssemblyZoneReady(Headlamp)`;
/// FSM transitions `PreparingToStart → Idle` without any manual injection.
#[tokio::test]
async fn given_power_on_when_headlamp_replies_ready_then_fsm_reaches_idle() {
    let (controller, _guard) = spawn_non_silent("STARTUP-1").await;

    controller.send_power_on().await.expect("power on");

    // No manual AssembliesReady injection — the wired barrier handles it.
    wait_fsm_state(&controller, FsmState::Idle, Duration::from_millis(500)).await;
}

// ── Test 2 ───────────────────────────────────────────────────────────────────

/// Silent headlamp never replies to `BecomeOn`; FSM must remain in `PreparingToStart`.
///
/// This test must NOT use `power_on_to_idle` (which would bypass the barrier).
/// It deliberately sends only `PowerOn` and then checks the state is still `PreparingToStart`.
#[tokio::test]
async fn given_power_on_with_silent_headlamp_then_fsm_stays_in_preparing_to_start() {
    let (controller, _guard) = spawn_silent("STARTUP-2").await;

    controller.send_power_on().await.expect("power on");

    // Give the actor time to process PowerOn and create the startup barrier.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let snapshot = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(50)))
        .await
        .expect("get snapshot");
    assert!(
        matches!(*snapshot.current_state(), FsmState::PreparingToStart { .. }),
        "silent headlamp must keep FSM in PreparingToStart; got {:?}",
        snapshot.current_state()
    );
}

// ── Test 3 ───────────────────────────────────────────────────────────────────

/// Non-silent headlamp replies to `BecomeOff`; brain commits `AssemblyZoneReady(Headlamp)`;
/// FSM transitions `PreparingToStop → Off` without any manual injection.
#[tokio::test]
async fn given_power_off_from_idle_when_headlamp_replies_off_then_fsm_reaches_off() {
    let (controller, _guard) = spawn_non_silent("SHUTDOWN-1").await;

    // Reach Idle via automatic BecomeOn flow.
    power_on_to_idle(&controller).await;

    controller.send_power_off().await.expect("power off");

    // No manual AssembliesStopped injection — the wired barrier handles it.
    wait_fsm_state(&controller, FsmState::Off, Duration::from_millis(500)).await;
}

// ── Test 4 ───────────────────────────────────────────────────────────────────

/// Silent headlamp never replies to `BecomeOff`; FSM must remain in `PreparingToStop`.
///
/// Boot via manual `ZoneReady` injection for the startup barrier (since headlamp is silent),
/// then send `PowerOff` and verify no automatic transition to `Off` occurs.
#[tokio::test]
async fn given_power_off_with_silent_headlamp_then_fsm_stays_in_preparing_to_stop() {
    let (controller, _guard) = spawn_silent("SHUTDOWN-2").await;

    // Boot: send PowerOn and manually inject the BecomeOn reply that the silent headlamp
    // will not send.
    controller.send_power_on().await.expect("power on");
    tokio::task::yield_now().await;
    inject_zone_ready(&controller, STARTUP_BARRIER_TURN, HeadlampState::Ready);
    wait_fsm_state(&controller, FsmState::Idle, Duration::from_millis(500)).await;

    // Now try to power off — silent headlamp will not reply to BecomeOff.
    controller.send_power_off().await.expect("power off");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let snapshot = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(50)))
        .await
        .expect("get snapshot");
    assert!(
        matches!(*snapshot.current_state(), FsmState::PreparingToStop { .. }),
        "silent headlamp must keep FSM in PreparingToStop; got {:?}",
        snapshot.current_state()
    );
}

// ── Test 5 ───────────────────────────────────────────────────────────────────

/// Ingress queued during startup must not commit until the barrier clears; post-start
/// telemetry commits FIFO from `Idle`, and RPM zero precedes `PowerOff`.
#[tokio::test]
async fn given_ingress_immediately_after_power_on_when_startup_unblocks_then_commits_fifo_from_idle()
 {
    let (transition_tx, mut transition_rx) = mpsc::channel(32);
    let opts = VehicleControllerRuntimeOptions {
        transition_tx: Some(transition_tx),
        test_silent_headlamp: true,
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("STARTUP-FIFO".to_string(), opts)
            .await
            .expect("spawn controller");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    controller.send_power_on().await.expect("power on");
    tokio::task::yield_now().await;

    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(900))
        .await
        .expect("lux during startup");
    controller
        .submit_twin_ingress(TwinIngressEvent::Telemetry(VssSignal::EngineRpm(1200)))
        .await
        .expect("drive during startup");
    controller
        .submit_twin_ingress(TwinIngressEvent::Telemetry(VssSignal::EngineRpm(0)))
        .await
        .expect("standstill during startup");
    controller
        .send_power_off()
        .await
        .expect("power off during startup");

    tokio::time::sleep(Duration::from_millis(25)).await;
    let blocked = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(50)))
        .await
        .expect("snapshot while startup blocked");
    assert!(matches!(
        blocked.current_state(),
        FsmState::PreparingToStart { .. }
    ));
    assert_ne!(blocked.context().visibility.ambient_lux, 900);
    assert_ne!(blocked.context().powertrain.primary_rpm(), 1200);

    let power_on = transition_rx.recv().await.expect("PowerOn row");
    assert_eq!(power_on.event, PublishedFsmEvent::PowerOn);
    assert!(
        transition_rx.try_recv().is_err(),
        "later turns must remain blocked"
    );

    inject_zone_ready(&controller, STARTUP_BARRIER_TURN, HeadlampState::Ready);
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    loop {
        let snapshot = controller
            .get_snapshot(Some(ractor::concurrency::Duration::from_millis(50)))
            .await
            .expect("snapshot while waiting for PowerOff turn");
        if matches!(snapshot.current_state(), FsmState::PreparingToStop(_)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for PreparingToStop; got {:?}",
            snapshot.current_state()
        );
        tokio::task::yield_now().await;
    }

    let mut rows = Vec::new();
    for _ in 0..6 {
        rows.push(
            transition_rx
                .recv()
                .await
                .expect("ordered startup/user row"),
        );
    }
    assert_eq!(rows[2].event, PublishedFsmEvent::UpdateAmbientLux(900));
    assert_eq!(rows[2].old_state, PublishedFsmState::Idle);
    assert_eq!(rows[3].event, PublishedFsmEvent::UpdateRpm(1200));
    assert_eq!(rows[3].next_state, PublishedFsmState::Driving);
    assert_eq!(rows[4].event, PublishedFsmEvent::UpdateRpm(0));
    assert_eq!(rows[4].next_state, PublishedFsmState::Idle);
    assert_eq!(rows[5].event, PublishedFsmEvent::PowerOff);
    assert!(matches!(
        rows[5].next_state,
        PublishedFsmState::PreparingToStop
    ));

    // The next turn after both startup barriers and the four queued ingress turns is
    // the first shutdown assembly barrier (headlamp).
    const QUEUED_SHUTDOWN_HEADLAMP_TURN: u64 =
        STARTUP_BARRIER_TURN + STARTUP_ASSEMBLY_BARRIER_COUNT + QUEUED_POST_POWER_ON_INGRESS_COUNT;
    inject_zone_ready(
        &controller,
        QUEUED_SHUTDOWN_HEADLAMP_TURN,
        HeadlampState::Off,
    );
    wait_fsm_state(&controller, FsmState::Off, Duration::from_millis(500)).await;
    let _headlamp_off = transition_rx.recv().await.expect("headlamp shutdown row");
    let final_off = transition_rx.recv().await.expect("wiper shutdown row");
    assert_eq!(final_off.next_state, PublishedFsmState::Off);
}
