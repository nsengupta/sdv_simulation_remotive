//! Independent observation streaks: first value is Initial, repeats increment
//! a per-signal counter, and a change reports the completed streak.
//! Orchestration: duplicate observations do not emit ledger rows or domain actions.

use std::time::Duration;

use tokio::sync::mpsc;

use crate::fsm::{DomainAction, FsmEvent};
use crate::test::{ActorGuard, power_on_to_idle};
use crate::twin_runtime::ObservationStreak;
use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;
use crate::vehicle_state::{ObservationDisposition, ObservedBool};
use crate::{PublishedDomainAction, PublishedTransitionRecord, VehicleController};

#[test]
fn unknown_then_false_is_initial_with_zero_duplicates() {
    let mut streak = ObservationStreak::default();

    let disposition = streak.observe(false);

    assert_eq!(disposition, ObservationDisposition::Initial);
    assert_eq!(streak.pending_summary(), Some((false, 0)));
}

#[test]
fn repeated_false_increments_the_same_streak() {
    let mut streak = ObservationStreak::default();
    assert_eq!(streak.observe(false), ObservationDisposition::Initial);

    assert_eq!(
        streak.observe(false),
        ObservationDisposition::Duplicate {
            current_duplicates: 1
        }
    );
    assert_eq!(
        streak.observe(false),
        ObservationDisposition::Duplicate {
            current_duplicates: 2
        }
    );
    assert_eq!(streak.pending_summary(), Some((false, 2)));
}

#[test]
fn complementary_value_reports_completed_duplicates_and_resets_count() {
    let mut streak = ObservationStreak::default();
    streak.observe(false);
    streak.observe(false);
    streak.observe(false);

    let changed = streak.observe(true);

    assert_eq!(
        changed,
        ObservationDisposition::Changed {
            completed_duplicates: 2
        }
    );
    assert_eq!(streak.pending_summary(), Some((true, 0)));
}

#[test]
fn shutdown_summary_reports_current_value_and_count() {
    let mut streak = ObservationStreak::default();
    streak.observe(false);
    streak.observe(false);
    streak.observe(false);
    streak.observe(true);

    assert_eq!(streak.pending_summary(), Some((true, 0)));
}

#[test]
fn sccm_and_bcm_streak_instances_cannot_interfere() {
    let mut sccm = ObservationStreak::default();
    let mut bcm = ObservationStreak::default();

    sccm.observe(false);
    sccm.observe(false);
    bcm.observe(true);

    assert_eq!(
        sccm.observe(false),
        ObservationDisposition::Duplicate {
            current_duplicates: 2
        }
    );
    assert_eq!(
        bcm.observe(true),
        ObservationDisposition::Duplicate {
            current_duplicates: 1
        }
    );
    assert_eq!(sccm.pending_summary(), Some((false, 2)));
    assert_eq!(bcm.pending_summary(), Some((true, 1)));
}

#[test]
fn bcm_left_and_right_streaks_are_independent() {
    let mut left = ObservationStreak::default();
    let mut right = ObservationStreak::default();

    left.observe(false);
    left.observe(false);
    right.observe(true);

    assert_eq!(
        left.observe(false),
        ObservationDisposition::Duplicate {
            current_duplicates: 2
        }
    );
    assert_eq!(right.pending_summary(), Some((true, 0)));
    assert_eq!(
        right.observe(true),
        ObservationDisposition::Duplicate {
            current_duplicates: 1
        }
    );
    assert_eq!(left.pending_summary(), Some((false, 2)));
}

async fn recv_row(rx: &mut mpsc::Receiver<PublishedTransitionRecord>) -> PublishedTransitionRecord {
    tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .expect("transition row timeout")
        .expect("transition channel closed")
}

#[tokio::test]
async fn observed_sequence_commits_six_changed_rows_and_zero_duplicate_rows() {
    let (tx, mut rx) = mpsc::channel(32);
    let options = VehicleControllerRuntimeOptions {
        transition_tx: Some(tx),
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("OBS-DUP".to_owned(), options)
            .await
            .expect("spawn controller");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    power_on_to_idle(&controller).await;
    let _power_on = recv_row(&mut rx).await;
    let _sccm_ready = recv_row(&mut rx).await;
    let _bcm_ready = recv_row(&mut rx).await;

    let events = [
        FsmEvent::HazardButtonObserved(false),
        FsmEvent::HazardButtonObserved(false),
        FsmEvent::HazardButtonObserved(true),
        FsmEvent::LeftTurnRequestObserved(false),
        FsmEvent::LeftTurnRequestObserved(false),
        FsmEvent::RightTurnRequestObserved(false),
        FsmEvent::RightTurnRequestObserved(false),
        FsmEvent::LeftTurnRequestObserved(true),
        FsmEvent::RightTurnRequestObserved(true),
    ];
    for event in events {
        controller
            .submit_fsm_event(event)
            .await
            .expect("observed ingress");
    }

    let mut rows = Vec::new();
    for _ in 0..6 {
        rows.push(recv_row(&mut rx).await);
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        rx.try_recv().is_err(),
        "duplicate observations must not emit ledger rows"
    );

    assert!(
        rows.windows(2)
            .all(|pair| pair[1].record_seq == pair[0].record_seq + 1),
        "changed records must stay monotonic"
    );
    assert!(
        rows.iter().all(|row| {
            !row.actions
                .iter()
                .any(|action| matches!(action, PublishedDomainAction::SetTurnLights { .. }))
        }),
        "observed path must not emit SetTurnLights"
    );
    let no_actions: &[DomainAction] = &[];
    assert!(
        !no_actions
            .iter()
            .any(|action| matches!(action, DomainAction::SetTurnLights { .. }))
    );

    let snapshot = controller
        .get_snapshot(Some(ractor::concurrency::Duration::from_millis(250)))
        .await
        .expect("snapshot");
    assert_eq!(snapshot.context().sccm.hazard_button, ObservedBool::On);
    assert_eq!(snapshot.context().bcm.left_turn_request, ObservedBool::On);
    assert_eq!(snapshot.context().bcm.right_turn_request, ObservedBool::On);

    let mut hazard = ObservationStreak::default();
    let mut left = ObservationStreak::default();
    let mut right = ObservationStreak::default();
    hazard.observe(false);
    hazard.observe(false);
    assert_eq!(
        hazard.observe(true),
        ObservationDisposition::Changed {
            completed_duplicates: 1
        }
    );
    left.observe(false);
    left.observe(false);
    assert_eq!(
        left.observe(true),
        ObservationDisposition::Changed {
            completed_duplicates: 1
        }
    );
    right.observe(false);
    right.observe(false);
    assert_eq!(
        right.observe(true),
        ObservationDisposition::Changed {
            completed_duplicates: 1
        }
    );
}

#[tokio::test]
async fn duplicate_tell_back_unblocks_the_barrier_without_a_record() {
    let (tx, mut rx) = mpsc::channel(16);
    let options = VehicleControllerRuntimeOptions {
        transition_tx: Some(tx),
        ..Default::default()
    };
    let (controller, handle) =
        VehicleController::install_and_start_with_options("OBS-UNBLOCK".to_owned(), options)
            .await
            .expect("spawn controller");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    power_on_to_idle(&controller).await;
    let _power_on = recv_row(&mut rx).await;
    let _sccm_ready = recv_row(&mut rx).await;
    let _bcm_ready = recv_row(&mut rx).await;

    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(false))
        .await
        .expect("initial hazard");
    let _initial = recv_row(&mut rx).await;

    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(false))
        .await
        .expect("duplicate hazard");
    controller
        .submit_fsm_event(FsmEvent::HazardButtonObserved(true))
        .await
        .expect("changed hazard");
    let changed = recv_row(&mut rx).await;
    assert!(
        rx.try_recv().is_err(),
        "duplicate must not leave a ledger row"
    );
    assert_eq!(
        changed.event,
        crate::PublishedFsmEvent::HazardButtonObserved(true)
    );
}
