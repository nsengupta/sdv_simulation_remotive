//! SCCM child actor: lifecycle readiness, hazard ownership, correlated tell-back.

use async_trait::async_trait;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use tokio::sync::mpsc;

use crate::digital_twin::TwinMessage;
use crate::fsm::AssemblyId;
use crate::test::ActorGuard;
use crate::twin_runtime::{SccmActor, SccmActorState, tell_sccm_zone};
use crate::vehicle_state::{ObservationDisposition, ObservedBool, SccmContext, SccmMessage};

#[derive(Default)]
struct ReadyCollector;

struct CollectorState {
    tx: mpsc::UnboundedSender<TwinMessage>,
}

#[async_trait]
impl Actor for ReadyCollector {
    type Msg = TwinMessage;
    type State = CollectorState;
    type Arguments = mpsc::UnboundedSender<TwinMessage>;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        tx: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(CollectorState { tx })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        let _ = state.tx.send(message);
        Ok(())
    }
}

async fn spawn_sccm(
    silent: bool,
) -> (
    ActorRef<crate::twin_runtime::SccmActorMsg>,
    ActorRef<TwinMessage>,
    mpsc::UnboundedReceiver<TwinMessage>,
    ActorGuard<crate::twin_runtime::SccmActorMsg>,
    ActorGuard<TwinMessage>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let (brain, brain_handle) = ractor::spawn::<ReadyCollector>(tx)
        .await
        .expect("collector");
    let (sccm, sccm_handle) = ractor::spawn::<SccmActor>(SccmActorState::new(
        SccmContext::default(),
        silent,
        brain.clone(),
    ))
    .await
    .expect("sccm actor");
    (
        sccm.clone(),
        brain.clone(),
        rx,
        ActorGuard {
            addr: sccm,
            handle: sccm_handle,
        },
        ActorGuard {
            addr: brain,
            handle: brain_handle,
        },
    )
}

async fn next_ready(rx: &mut mpsc::UnboundedReceiver<TwinMessage>) -> TwinMessage {
    tokio::time::timeout(std::time::Duration::from_millis(250), rx.recv())
        .await
        .expect("sccm tell-back timeout")
        .expect("collector closed")
}

#[tokio::test]
async fn sccm_become_on_tells_back_lifecycle_readiness() {
    let (sccm, _brain, mut rx, _sccm_guard, _brain_guard) = spawn_sccm(false).await;

    tell_sccm_zone(&sccm, 2, 0, SccmMessage::BecomeOn).expect("tell become on");
    let TwinMessage::ZoneReady {
        zone_id,
        turn_id,
        tell_attempt,
        reply,
    } = next_ready(&mut rx).await
    else {
        panic!("expected ZoneReady");
    };

    assert_eq!(zone_id, AssemblyId::Sccm);
    assert_eq!(turn_id, 2);
    assert_eq!(tell_attempt, 0);
    let sccm_reply = reply.as_sccm().expect("sccm reply");
    assert_eq!(sccm_reply.disposition, ObservationDisposition::Lifecycle);
    assert_eq!(sccm_reply.ctx.hazard_button, ObservedBool::Unknown);
}

#[tokio::test]
async fn sccm_hazard_tell_back_is_correlated_and_initial() {
    let (sccm, _brain, mut rx, _sccm_guard, _brain_guard) = spawn_sccm(false).await;

    tell_sccm_zone(&sccm, 11, 0, SccmMessage::HazardButtonObserved(false)).expect("tell hazard");
    let TwinMessage::ZoneReady {
        zone_id,
        turn_id,
        tell_attempt,
        reply,
    } = next_ready(&mut rx).await
    else {
        panic!("expected ZoneReady");
    };

    assert_eq!(zone_id, AssemblyId::Sccm);
    assert_eq!(turn_id, 11);
    assert_eq!(tell_attempt, 0);
    let sccm_reply = reply.as_sccm().expect("sccm reply");
    assert_eq!(sccm_reply.disposition, ObservationDisposition::Initial);
    assert_eq!(sccm_reply.ctx.hazard_button, ObservedBool::Off);
}

#[tokio::test]
async fn sccm_duplicate_still_tells_back_unchanged_context() {
    let (sccm, _brain, mut rx, _sccm_guard, _brain_guard) = spawn_sccm(false).await;

    tell_sccm_zone(&sccm, 1, 0, SccmMessage::HazardButtonObserved(false)).expect("initial");
    let _ = next_ready(&mut rx).await;

    tell_sccm_zone(&sccm, 2, 0, SccmMessage::HazardButtonObserved(false)).expect("duplicate");
    let TwinMessage::ZoneReady { reply, .. } = next_ready(&mut rx).await else {
        panic!("expected duplicate ZoneReady");
    };
    let sccm_reply = reply.as_sccm().expect("sccm reply");
    assert_eq!(
        sccm_reply.disposition,
        ObservationDisposition::Duplicate {
            current_duplicates: 1
        }
    );
    assert_eq!(sccm_reply.ctx.hazard_button, ObservedBool::Off);
}

#[tokio::test]
async fn sccm_change_reports_completed_streak_and_resets() {
    let (sccm, _brain, mut rx, _sccm_guard, _brain_guard) = spawn_sccm(false).await;

    for turn_id in 1..=3 {
        tell_sccm_zone(&sccm, turn_id, 0, SccmMessage::HazardButtonObserved(false))
            .expect("false observation");
        let _ = next_ready(&mut rx).await;
    }

    tell_sccm_zone(&sccm, 4, 0, SccmMessage::HazardButtonObserved(true)).expect("changed");
    let TwinMessage::ZoneReady { reply, .. } = next_ready(&mut rx).await else {
        panic!("expected changed ZoneReady");
    };
    let sccm_reply = reply.as_sccm().expect("sccm reply");
    assert_eq!(
        sccm_reply.disposition,
        ObservationDisposition::Changed {
            completed_duplicates: 2
        }
    );
    assert_eq!(sccm_reply.ctx.hazard_button, ObservedBool::On);
}

#[tokio::test]
async fn sccm_shutdown_summary_reports_remaining_hazard_streak() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let (brain, _) = ractor::spawn::<ReadyCollector>(tx)
        .await
        .expect("collector");
    let mut state = SccmActorState::new(SccmContext::default(), false, brain.clone());
    state.hazard_streak.observe(false);
    state.hazard_streak.observe(false);
    state.hazard_streak.observe(true);

    assert_eq!(state.hazard_streak.pending_summary(), Some((true, 0)));
    brain.stop(None);
}
