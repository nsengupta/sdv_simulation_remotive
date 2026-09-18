//! L4 wiper twinlet — brain **tell**s [`WiperActorMsg::Apply`]; twinlet **tell**s
//! [`TwinMessage::ZoneReady`] immediately (no ACK protocol).
//!
//! all wiper transitions are direct — no `OffRequested`/`OnRequested` intermediate
//! states, no ACK timer. `post_stop` is a no-op.

use async_trait::async_trait;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use std::time::Instant;

use crate::digital_twin::{TwinMessage, ZoneReply};
use crate::vehicle_state::{WiperContext, WiperMessage};

/// Tell payload: one [`WiperMessage`] for this brain [`turn_id`](Self::turn_id).
#[derive(Debug)]
pub struct WiperActorVocabulary {
    pub message: WiperMessage,
    pub now: Instant,
    pub turn_id: u64,
    /// Matches brain tell-back wait attempt (retries use incrementing ids).
    pub tell_attempt: u32,
}

/// Wiper twinlet mailbox — brain tells only (no ACK deadline variant).
#[derive(Debug)]
pub enum WiperActorMsg {
    Apply(WiperActorVocabulary),
}

#[derive(Debug)]
pub struct WiperActorState {
    pub ctx: WiperContext,
    /// When true, swallow tells without tell-back (contract tests only).
    pub silent: bool,
    brain: ActorRef<TwinMessage>,
}

impl WiperActorState {
    pub fn new(ctx: WiperContext, silent: bool, brain: ActorRef<TwinMessage>) -> Self {
        Self { ctx, silent, brain }
    }
}

#[derive(Default)]
pub struct WiperActor;

#[async_trait]
impl Actor for WiperActor {
    type Msg = WiperActorMsg;
    type State = WiperActorState;
    type Arguments = WiperActorState;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(args)
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        let WiperActorMsg::Apply(vocab) = message;
        Self::handle_apply(state, vocab).await
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        // No timers to abort — no-op.
        Ok(())
    }
}

impl WiperActor {
    async fn handle_apply(
        state: &mut WiperActorState,
        WiperActorVocabulary {
            message,
            now: _now,
            turn_id,
            tell_attempt,
        }: WiperActorVocabulary,
    ) -> Result<(), ActorProcessingErr> {
        if state.silent {
            return Ok(());
        }

        let zone_reply = state.ctx.on_receiving_message(message);
        state.ctx = zone_reply.ctx.clone();
        state
            .brain
            .send_message(TwinMessage::ZoneReady {
                zone_id: crate::fsm::AssemblyId::Wiper,
                turn_id,
                tell_attempt,
                reply: ZoneReply::Wiper(zone_reply),
            })
            .map_err(|e| {
                ActorProcessingErr::from(std::io::Error::other(format!(
                    "WiperActor ZoneReady tell-back: {e:?}"
                )))
            })?;
        Ok(())
    }
}

/// Fire-and-forget tell to the wiper twinlet (no reply port on this hop).
pub fn tell_wiper_zone(
    wiper: &ActorRef<WiperActorMsg>,
    turn_id: u64,
    tell_attempt: u32,
    message: WiperMessage,
    now: Instant,
) -> Result<(), ActorProcessingErr> {
    wiper
        .send_message(WiperActorMsg::Apply(WiperActorVocabulary {
            message,
            now,
            turn_id,
            tell_attempt,
        }))
        .map_err(|e| {
            ActorProcessingErr::from(std::io::Error::other(format!("tell_wiper_zone: {e:?}")))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsm::AssemblyId;
    use crate::vehicle_state::WiperState;
    use tokio::sync::mpsc;

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

    #[tokio::test]
    async fn tell_backs_use_the_parent_brain_captured_at_construction() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (brain, _brain_handle) = ractor::spawn::<ReadyCollector>(tx)
            .await
            .expect("collector");
        let (wiper, _wiper_handle) = ractor::spawn::<WiperActor>(WiperActorState::new(
            WiperContext {
                state: WiperState::Ready,
            },
            false,
            brain.clone(),
        ))
        .await
        .expect("wiper actor");

        tell_wiper_zone(&wiper, 5, 2, WiperMessage::BecomeOn, Instant::now())
            .expect("tell become on");
        let TwinMessage::ZoneReady {
            zone_id,
            turn_id,
            tell_attempt,
            ..
        } = tokio::time::timeout(std::time::Duration::from_millis(250), rx.recv())
            .await
            .expect("zone ready timeout")
            .expect("collector closed")
        else {
            panic!("expected ZoneReady tell-back");
        };
        assert_eq!(zone_id, AssemblyId::Wiper);
        assert_eq!(turn_id, 5);
        assert_eq!(tell_attempt, 2);

        tell_wiper_zone(&wiper, 6, 0, WiperMessage::BecomeOff, Instant::now())
            .expect("tell become off");
        let TwinMessage::ZoneReady {
            zone_id, turn_id, ..
        } = tokio::time::timeout(std::time::Duration::from_millis(250), rx.recv())
            .await
            .expect("second tell-back timeout")
            .expect("collector closed")
        else {
            panic!("expected second ZoneReady tell-back");
        };
        assert_eq!(zone_id, AssemblyId::Wiper);
        assert_eq!(turn_id, 6);

        brain.stop(None);
        wiper.stop(None);
    }
}
