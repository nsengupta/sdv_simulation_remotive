//! `VecDeque<TurnBarrier>` reorder-buffer: ordering invariants (RED → GREEN).
//!
//! ## Why an older single-slot design fails these
//!
//! An older design used `pending_turn + fsm_backlog`: a second `Fsm` event arriving while a zone
//! tell is in flight sits in `fsm_backlog` **without a turn_id**. A manually injected
//! `ZoneReady { turn_id: N+1 }` does not match `pending_turn { turn_id: N }` and is
//! **dropped**. After the front turn resolves, `pump_fsm_backlog` starts the second
//! event with the SAME `turn_id` (N+1) but a fresh zone tell to the silent headlamp;
//! eventually all retries exhaust and a synthetic reply is committed, which carries a
//! `LogWarning` action.
//!
//! The reorder buffer gives every `Fsm` event its own `TurnBarrier` immediately. The injected
//! `ZoneReply` is stored in that barrier; when the front barrier drains, the rear one
//! drains with its **real** reply — no synthetic, no `LogWarning`.
//!
//! ## RED assertion
//!
//! `rows[1].actions.is_empty` — previously the rear event's synthetic reply injects
//! `PublishedDomainAction::LogWarning`; previously the real injected reply has no outcomes.
//!
//! ## Timing discipline
//!
//! `ZONE_TELL_BACK_WAIT` in test mode is 50 ms. All manual injections happen within
//! ~5 ms of event submission (well before the first retry), so `tell_attempt` is still 0
//! in both the actor's wait state and our injected message.

use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::VehicleController;
use crate::digital_twin::{TwinMessage, ZoneReply};
use crate::fsm::{AssemblyId, FsmEvent, FsmState, HeadlampState};
use crate::observation_records::transition::{PublishedDomainAction, PublishedTransitionRecord};
use crate::test::ActorGuard;
use crate::twin_runtime::controller::vehicle_controller::{
    AssemblyTopology, VehicleControllerRuntimeOptions,
};
use crate::vehicle_physics::LUX_ON_THRESHOLD;
use crate::vehicle_state::{HeadlampContext, HeadlampOutcome, HeadlampZoneReply};

// ── helpers ─────────────────────────────────────────────────────────────────

fn zone_reply(state: HeadlampState) -> ZoneReply {
    ZoneReply::Headlamp(HeadlampZoneReply {
        ctx: HeadlampContext {
            state,
            ack_pending_since: if matches!(
                state,
                HeadlampState::OnRequested | HeadlampState::OffRequested
            ) {
                Some(Instant::now())
            } else {
                None
            },
        },
        outcomes: vec![],
    })
}

async fn drain_n(
    rx: &mut mpsc::Receiver<PublishedTransitionRecord>,
    n: usize,
    timeout: Duration,
) -> Vec<PublishedTransitionRecord> {
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        match tokio::time::timeout(timeout, rx.recv()).await {
            Ok(Some(r)) => rows.push(r),
            Ok(None) => panic!("transition channel closed (row {}/{})", i + 1, n),
            Err(_) => panic!("timeout waiting for row {}/{}", i + 1, n),
        }
    }
    rows
}

async fn assert_no_row(rx: &mut mpsc::Receiver<PublishedTransitionRecord>, window: Duration) {
    match tokio::time::timeout(window, rx.recv()).await {
        Ok(Some(r)) => panic!("unexpected early commit: {:?}", r.event),
        Ok(None) => panic!("transition channel closed"),
        Err(_) => {} // nothing arrived — correct
    }
}

fn inject_zone_ready(controller: &VehicleController, turn_id: u64, state: HeadlampState) {
    inject_zone_ready_with(controller, turn_id, 0, state, Vec::<HeadlampOutcome>::new());
}

fn inject_zone_ready_with(
    controller: &VehicleController,
    turn_id: u64,
    tell_attempt: u32,
    state: HeadlampState,
    outcomes: Vec<HeadlampOutcome>,
) {
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Headlamp,
            turn_id,
            tell_attempt,
            reply: ZoneReply::Headlamp(HeadlampZoneReply {
                ctx: HeadlampContext {
                    state,
                    ack_pending_since: None,
                },
                outcomes,
            }),
        })
        .expect("inject_zone_ready");
}

fn inject_timeout(controller: &VehicleController, turn_id: u64, attempt: u32) {
    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneTellBackTimeout {
            zone_id: AssemblyId::Headlamp,
            turn_id,
            tell_attempt: attempt,
        })
        .expect("inject_timeout");
}

/// Spawn a silent-headlamp controller with a fresh transition channel.
///
/// `initial_headlamp_ctx` is intentionally omitted: sets the headlamp
/// to `Ready` automatically when `boot_silent` injects the `BecomeOn` zone reply
/// for the startup barrier (turn 2). Removing the override ensures that the actor
/// exercises the real `BecomeOn` path during boot.
async fn spawn_silent(
    identity: &str,
) -> (
    VehicleController,
    mpsc::Receiver<PublishedTransitionRecord>,
    ActorGuard<TwinMessage>,
) {
    let (tx, rx) = mpsc::channel(32);
    let opts = VehicleControllerRuntimeOptions {
        assembly_topology: AssemblyTopology::Legacy,
        transition_tx: Some(tx),
        test_silent_headlamp: true, // suppress real headlamp replies; we inject manually
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options(identity.to_string(), opts)
            .await
            .unwrap();
    let guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };
    (controller, rx, guard)
}

/// First turn ID available to user-driven events after `boot_silent`.
/// PowerOn=1, BCM startup=2, first user event=3.
const FIRST_USER_TURN: u64 = 3;

/// Boot sequence with BCM as the sole lifecycle participant.
async fn boot_silent(
    controller: &VehicleController,
    rx: &mut mpsc::Receiver<PublishedTransitionRecord>,
) {
    controller.send_power_on().await.expect("power on");
    crate::test::wait_fsm_state(
        controller,
        FsmState::Idle,
        std::time::Duration::from_millis(500),
    )
    .await;
    drain_n(rx, 2, std::time::Duration::from_secs(3)).await;
}

#[tokio::test]
async fn stale_timeout_after_retry_does_not_resolve_or_commit_barrier() {
    let (controller, mut rx, _guard) = spawn_silent("ROB-STALE-TIMEOUT").await;
    boot_silent(&controller, &mut rx).await;

    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(20))
        .await
        .expect("headlamp turn");
    tokio::task::yield_now().await;

    inject_timeout(&controller, FIRST_USER_TURN, 0);
    tokio::task::yield_now().await;
    inject_timeout(&controller, FIRST_USER_TURN, 0);
    assert_no_row(&mut rx, Duration::from_millis(20)).await;

    controller
        .get_actor_ref()
        .send_message(TwinMessage::ZoneReady {
            zone_id: AssemblyId::Headlamp,
            turn_id: FIRST_USER_TURN,
            tell_attempt: 1,
            reply: zone_reply(HeadlampState::Ready),
        })
        .expect("current-attempt reply");
    let rows = drain_n(&mut rx, 1, Duration::from_secs(1)).await;
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0]
            .actions
            .iter()
            .all(|action| !matches!(action, PublishedDomainAction::LogWarning(_)))
    );
}

#[tokio::test]
async fn duplicate_same_attempt_reply_cannot_overwrite_accepted_reply() {
    let (controller, mut rx, _guard) = spawn_silent("ROB-DUPLICATE-REPLY").await;
    boot_silent(&controller, &mut rx).await;

    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(20))
        .await
        .expect("front turn");
    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(10))
        .await
        .expect("rear turn");
    tokio::task::yield_now().await;

    inject_zone_ready_with(
        &controller,
        FIRST_USER_TURN + 1,
        0,
        HeadlampState::OnRequested,
        vec![HeadlampOutcome::RequestOn],
    );
    inject_zone_ready(&controller, FIRST_USER_TURN + 1, HeadlampState::Ready);
    inject_zone_ready(&controller, FIRST_USER_TURN, HeadlampState::Ready);

    let rows = drain_n(&mut rx, 2, Duration::from_secs(1)).await;
    assert!(
        rows[1]
            .actions
            .contains(&PublishedDomainAction::RequestFrontHeadlampOn),
        "duplicate reply replaced the first accepted outcome: {:?}",
        rows[1].actions
    );
}

#[tokio::test]
async fn matching_timeout_after_accepted_reply_cannot_retry_or_alter_reply() {
    let (controller, mut rx, _guard) = spawn_silent("ROB-TIMEOUT-AFTER-REPLY").await;
    boot_silent(&controller, &mut rx).await;

    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(20))
        .await
        .expect("front turn");
    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(10))
        .await
        .expect("rear turn");
    tokio::task::yield_now().await;

    inject_zone_ready_with(
        &controller,
        FIRST_USER_TURN + 1,
        0,
        HeadlampState::OnRequested,
        vec![HeadlampOutcome::RequestOn],
    );
    inject_timeout(&controller, FIRST_USER_TURN + 1, 0);
    inject_zone_ready_with(
        &controller,
        FIRST_USER_TURN + 1,
        1,
        HeadlampState::Ready,
        vec![],
    );
    inject_zone_ready(&controller, FIRST_USER_TURN, HeadlampState::Ready);

    let rows = drain_n(&mut rx, 2, Duration::from_secs(1)).await;
    assert!(
        rows[1]
            .actions
            .contains(&PublishedDomainAction::RequestFrontHeadlampOn),
        "post-reply timeout retried and allowed replacement: {:?}",
        rows[1].actions
    );
}

// ── Test 1 ──────────────────────────────────────────────────────────────────

/// Two zone-directed events; rear-barrier reply arrives before front-barrier reply.
///
/// `ZoneReady(4)` is stored in `barrier(4)`. Nothing drains until
/// `ZoneReady(3)` completes `barrier(3)`. Both then drain in FIFO order; `rows[1]`
/// carries the real injected reply → `actions` is empty.
///
/// `ZoneReady(4)` is dropped (pending turn is turn 3). After
/// `ZoneReady(3)` commits turn 3, `pump_fsm_backlog` starts lux2 with a fresh zone
/// tell to the silent headlamp; retries exhaust → synthetic reply → `rows[1].actions`
/// contains `LogWarning` → assertion fails.
#[tokio::test]
async fn two_zone_directed_events_commit_in_arrival_order() {
    let (controller, mut rx, _guard) = spawn_silent("ROB-ORDER-1").await;

    boot_silent(&controller, &mut rx).await;

    // Turn 3: zone-directed (headlamp zone tell needed).
    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(20))
        .await
        .unwrap();
    // Turn 4: also zone-directed.
    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(LUX_ON_THRESHOLD + 100))
        .await
        .unwrap();

    // Inject turn-4 reply FIRST — before the 50 ms retry timer fires (all injections at ~5 ms).
    // stores it; drops it (pending = turn 3).
    tokio::task::yield_now().await;
    inject_zone_ready(&controller, FIRST_USER_TURN + 1, HeadlampState::Ready);

    // Front barrier (turn 3) still pending — nothing must drain.
    assert_no_row(&mut rx, Duration::from_millis(30)).await;

    // Inject turn-3 reply → front completes → drain: turn 3 then turn 4.
    inject_zone_ready(&controller, FIRST_USER_TURN, HeadlampState::Ready);

    let rows = drain_n(&mut rx, 2, Duration::from_secs(3)).await;
    assert_eq!(rows.len(), 2, "both events must commit");
    assert!(
        rows[0].record_seq < rows[1].record_seq,
        "turn 3 must precede turn 4 in the ledger"
    );
    // RED assertion: → injected reply (no LogWarning); → synthetic (LogWarning).
    assert!(
        rows[1]
            .actions
            .iter()
            .all(|a| !matches!(a, PublishedDomainAction::LogWarning(_))),
        "rear barrier must commit with real reply (no LogWarning), got {:?}",
        rows[1].actions
    );
}

// ── Test 2 ──────────────────────────────────────────────────────────────────

/// Three events (zone, zone, non-zone); zone replies arrive out of order.
///
/// `barrier(5=UpdateRpm)` is immediately complete; after both zone
/// replies are stored and the front drains, all three commit in order with no synthetic.
///
/// `ZoneReady(4)` dropped; lux2 restarts via pump and exhausts via timer;
/// `rows[1]` is synthetic (LogWarning) → assertion fails.
#[tokio::test]
async fn three_events_drain_in_arrival_order_when_zone_replies_arrive_out_of_order() {
    use crate::vehicle_physics::RPM_DRIVING_THRESHOLD;

    let (controller, mut rx, _guard) = spawn_silent("ROB-ORDER-2").await;

    boot_silent(&controller, &mut rx).await;

    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(20))
        .await
        .unwrap(); // turn 3, zone
    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(LUX_ON_THRESHOLD + 100))
        .await
        .unwrap(); // turn 4, zone
    controller
        .submit_fsm_event(FsmEvent::UpdateRpm(RPM_DRIVING_THRESHOLD + 100))
        .await
        .unwrap(); // turn 5, no zone → immediately complete

    tokio::task::yield_now().await;

    inject_zone_ready(&controller, FIRST_USER_TURN + 1, HeadlampState::Ready); // turn 4
    assert_no_row(&mut rx, Duration::from_millis(30)).await;
    inject_zone_ready(&controller, FIRST_USER_TURN, HeadlampState::Ready); // turn 3

    let rows = drain_n(&mut rx, 3, Duration::from_secs(3)).await;
    assert_eq!(rows.len(), 3, "all three events must commit");
    assert!(rows[0].record_seq < rows[1].record_seq);
    assert!(rows[1].record_seq < rows[2].record_seq);
    // RED assertion: turn 4 (rows[1]) must use the real reply, not a synthetic.
    assert!(
        rows[1]
            .actions
            .iter()
            .all(|a| !matches!(a, PublishedDomainAction::LogWarning(_))),
        "turn-4 barrier must commit with real reply (no LogWarning), got {:?}",
        rows[1].actions
    );
}

// ── Test 3 ──────────────────────────────────────────────────────────────────

/// Manually exhaust front-barrier retries; rear barrier had a real reply stored
/// before exhaustion occurred.
///
/// `ZoneReady(4)` stored in `barrier(4)` before any timeout fires.
/// Injected timeouts exhaust `barrier(3)` → synthetic commit. Drain loop immediately
/// finds `barrier(4)` complete → commits with stored real reply → `rows[1].actions` empty.
///
/// `ZoneReady(4)` dropped. After turn 3 exhausts, `pump_fsm_backlog`
/// restarts lux2 with a fresh zone tell; headlamp is silent; lux2 exhausts via timer →
/// `rows[1]` is synthetic → `LogWarning` present → assertion fails.
#[tokio::test]
async fn exhausted_front_barrier_unblocks_rear_with_stored_reply() {
    use crate::twin_runtime::constants::ZONE_TELL_BACK_MAX_RETRIES;

    let (controller, mut rx, _guard) = spawn_silent("ROB-TIMEOUT-1").await;

    boot_silent(&controller, &mut rx).await;

    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(20))
        .await
        .unwrap(); // turn 3, zone
    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(LUX_ON_THRESHOLD + 100))
        .await
        .unwrap(); // turn 4, zone

    tokio::task::yield_now().await;

    // Store turn-4 reply BEFORE exhausting turn 3.
    // stored in barrier(4). : dropped (pending = turn 3).
    inject_zone_ready(&controller, FIRST_USER_TURN + 1, HeadlampState::Ready);

    // Manually exhaust turn-3 retries (attempt 0 → retry → 1 → retry → 2 → gave up).
    for attempt in 0..=(ZONE_TELL_BACK_MAX_RETRIES as u32) {
        inject_timeout(&controller, FIRST_USER_TURN, attempt);
    }

    // Turn 3 committed (synthetic). Turn 4 committed (real in ; synthetic in ).
    let rows = drain_n(&mut rx, 2, Duration::from_secs(3)).await;
    assert_eq!(rows.len(), 2);
    assert!(rows[0].record_seq < rows[1].record_seq);
    // RED assertion: rows[1] must use real reply (no LogWarning).
    assert!(
        rows[1]
            .actions
            .iter()
            .all(|a| !matches!(a, PublishedDomainAction::LogWarning(_))),
        "rear barrier must use stored real reply (no LogWarning), got {:?}",
        rows[1].actions
    );
}

// ── Test 4 ──────────────────────────────────────────────────────────────────

/// Drain stops when the front barrier is incomplete; only advances when front is resolved.
///
/// `barrier(4=UpdateRpm)` is immediately complete but blocked by
/// incomplete `barrier(3=lux)`. `assert_no_row` verifies nothing drains prematurely.
/// `ZoneReady(3)` completes the front → both drain. `rows[1]` (UpdateRpm) has no zone
/// reply → `actions` empty. The key property is that `rows[0]` (lux) must also carry no
/// `LogWarning` (this path uses the injected `ZoneReady(3)` directly).
///
/// To make the older single-slot design fail: also send a second zone event (turn 4 = lux2)
/// so that an injected `ZoneReady(4)` before the front resolves is dropped.
#[tokio::test]
async fn second_zone_reply_before_first_does_not_drain_anything_prematurely() {
    let (controller, mut rx, _guard) = spawn_silent("ROB-DRAIN-1").await;

    boot_silent(&controller, &mut rx).await;

    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(20))
        .await
        .unwrap(); // turn 3, zone
    controller
        .submit_fsm_event(FsmEvent::UpdateAmbientLux(LUX_ON_THRESHOLD + 100))
        .await
        .unwrap(); // turn 4, zone

    tokio::task::yield_now().await;

    // Inject turn-4 reply first — drops it; stores it.
    inject_zone_ready(&controller, FIRST_USER_TURN + 1, HeadlampState::Ready);

    // Front (turn 3) still incomplete → nothing must drain yet.
    assert_no_row(&mut rx, Duration::from_millis(30)).await;

    // Complete the front → drain: turn 3 then turn 4.
    inject_zone_ready(&controller, FIRST_USER_TURN, HeadlampState::Ready);

    let rows = drain_n(&mut rx, 2, Duration::from_secs(3)).await;
    assert_eq!(rows.len(), 2);
    assert!(rows[0].record_seq < rows[1].record_seq);
    // RED assertion: real reply stored for turn 4 → no LogWarning in rows[1].
    assert!(
        rows[1]
            .actions
            .iter()
            .all(|a| !matches!(a, PublishedDomainAction::LogWarning(_))),
        "turn-4 must commit with stored real reply (no LogWarning), got {:?}",
        rows[1].actions
    );
}
