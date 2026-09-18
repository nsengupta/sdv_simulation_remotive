//! L4 headlamp twinlet — brain **tell**s [`HeadlampActorVocabulary::Apply`]; twinlet **tell**s
//! [`TwinMessage::ZoneReady`] (correlated) or
//! [`TwinMessage::ZoneSpontaneous`] (ACK timer).

use async_trait::async_trait;
use ractor::concurrency::{Duration as RactorDuration, JoinHandle};
use ractor::{Actor, ActorProcessingErr, ActorRef, MessagingErr};
use std::time::Instant;

use crate::digital_twin::{TwinMessage, ZoneReply, ZoneSpontaneousEvent};
use crate::fsm::{FrontHeadlampIncompleteCause, FrontHeadlampSwitchDirection};
use crate::vehicle_physics::{FRONT_HEADLAMP_OFF_ACK_WAIT, FRONT_HEADLAMP_ON_ACK_WAIT};
use crate::vehicle_state::{HeadlampContext, HeadlampMessage, HeadlampState};

type AckTimer = JoinHandle<Result<(), MessagingErr<HeadlampActorMsg>>>;

/// Tell payload: one [`HeadlampMessage`] for this brain [`turn_id`](Self::turn_id).
#[derive(Debug)]
pub struct HeadlampActorVocabulary {
    pub message: HeadlampMessage,
    pub now: Instant,
    pub turn_id: u64,
    /// Matches brain tell-back wait attempt (retries use incrementing ids).
    pub tell_attempt: u32,
}

/// Headlamp twinlet mailbox — brain tells plus internal ACK deadlines.
#[derive(Debug)]
pub enum HeadlampActorMsg {
    Apply(HeadlampActorVocabulary),
    AckWaitElapsed {
        direction: FrontHeadlampSwitchDirection,
    },
}

#[derive(Debug)]
pub struct HeadlampActorState {
    pub ctx: HeadlampContext,
    /// When true, swallow tells without tell-back (contract tests only).
    pub silent: bool,
    brain: ActorRef<TwinMessage>,
    ack_timer: Option<AckTimer>,
}

impl HeadlampActorState {
    pub fn new(ctx: HeadlampContext, silent: bool, brain: ActorRef<TwinMessage>) -> Self {
        Self {
            ctx,
            silent,
            brain,
            ack_timer: None,
        }
    }
}

#[derive(Default)]
pub struct HeadlampActor;

#[async_trait]
impl Actor for HeadlampActor {
    type Msg = HeadlampActorMsg;
    type State = HeadlampActorState;
    type Arguments = HeadlampActorState;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(args)
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            HeadlampActorMsg::Apply(vocab) => {
                Self::handle_apply(&myself, state, vocab).await?;
            }
            HeadlampActorMsg::AckWaitElapsed { direction } => {
                Self::handle_ack_wait_elapsed(state, direction)?;
            }
        }
        Ok(())
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        abort_ack_timer(&mut state.ack_timer);
        Ok(())
    }
}

impl HeadlampActor {
    async fn handle_apply(
        myself: &ActorRef<HeadlampActorMsg>,
        state: &mut HeadlampActorState,
        HeadlampActorVocabulary {
            message,
            now,
            turn_id,
            tell_attempt,
        }: HeadlampActorVocabulary,
    ) -> Result<(), ActorProcessingErr> {
        if state.silent {
            return Ok(());
        }

        let zone_reply = state.ctx.on_receiving_message(message, now);
        state.ctx = zone_reply.ctx.clone();
        maybe_arm_ack_timer(myself, state);
        state
            .brain
            .send_message(TwinMessage::ZoneReady {
                zone_id: crate::fsm::AssemblyId::Headlamp,
                turn_id,
                tell_attempt,
                reply: ZoneReply::Headlamp(zone_reply),
            })
            .map_err(|e| {
                ActorProcessingErr::from(std::io::Error::other(format!(
                    "ZoneReady tell-back: {e:?}"
                )))
            })?;
        Ok(())
    }

    fn handle_ack_wait_elapsed(
        state: &mut HeadlampActorState,
        direction: FrontHeadlampSwitchDirection,
    ) -> Result<(), ActorProcessingErr> {
        state.ack_timer = None;
        let now = Instant::now();
        let zone_reply = state.ctx.on_receiving_message(
            HeadlampMessage::ActuationIncomplete {
                direction,
                cause: FrontHeadlampIncompleteCause::TimedOut,
            },
            now,
        );
        state.ctx = zone_reply.ctx.clone();
        abort_ack_timer(&mut state.ack_timer);
        state
            .brain
            .send_message(TwinMessage::ZoneSpontaneous {
                zone_id: crate::fsm::AssemblyId::Headlamp,
                event: ZoneSpontaneousEvent::Headlamp {
                    direction,
                    cause: FrontHeadlampIncompleteCause::TimedOut,
                    reply: zone_reply,
                },
            })
            .map_err(|e| {
                ActorProcessingErr::from(std::io::Error::other(format!(
                    "ZoneSpontaneous tell-back: {e:?}"
                )))
            })?;
        Ok(())
    }
}

fn abort_ack_timer(timer: &mut Option<AckTimer>) {
    if let Some(handle) = timer.take() {
        handle.abort();
    }
}

fn maybe_arm_ack_timer(myself: &ActorRef<HeadlampActorMsg>, state: &mut HeadlampActorState) {
    abort_ack_timer(&mut state.ack_timer);
    if state.ctx.ack_pending_since.is_none() {
        return;
    }
    let (direction, wait) = match state.ctx.state {
        HeadlampState::OnRequested => {
            (FrontHeadlampSwitchDirection::On, FRONT_HEADLAMP_ON_ACK_WAIT)
        }
        HeadlampState::OffRequested => (
            FrontHeadlampSwitchDirection::Off,
            FRONT_HEADLAMP_OFF_ACK_WAIT,
        ),
        _ => return,
    };
    state.ack_timer = Some(myself.send_after(RactorDuration::from(wait), move || {
        HeadlampActorMsg::AckWaitElapsed { direction }
    }));
}

/// Fire-and-forget tell to the headlamp twinlet (no reply port on this hop).
pub fn tell_headlamp_zone(
    headlamp: &ActorRef<HeadlampActorMsg>,
    turn_id: u64,
    tell_attempt: u32,
    message: HeadlampMessage,
    now: Instant,
) -> Result<(), ActorProcessingErr> {
    headlamp
        .send_message(HeadlampActorMsg::Apply(HeadlampActorVocabulary {
            message,
            now,
            turn_id,
            tell_attempt,
        }))
        .map_err(|e| {
            ActorProcessingErr::from(std::io::Error::other(format!("tell_headlamp_zone: {e:?}")))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsm::AssemblyId;
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
        let (headlamp, _headlamp_handle) = ractor::spawn::<HeadlampActor>(HeadlampActorState::new(
            HeadlampContext {
                state: crate::vehicle_state::HeadlampState::Ready,
                ack_pending_since: None,
            },
            false,
            brain.clone(),
        ))
        .await
        .expect("headlamp actor");

        tell_headlamp_zone(&headlamp, 3, 1, HeadlampMessage::BecomeOn, Instant::now())
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
        assert_eq!(zone_id, AssemblyId::Headlamp);
        assert_eq!(turn_id, 3);
        assert_eq!(tell_attempt, 1);

        tell_headlamp_zone(&headlamp, 4, 0, HeadlampMessage::BecomeOff, Instant::now())
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
        assert_eq!(zone_id, AssemblyId::Headlamp);
        assert_eq!(turn_id, 4);

        brain.stop(None);
        headlamp.stop(None);
    }
}
