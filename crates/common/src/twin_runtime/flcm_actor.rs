use std::time::Duration;

use async_trait::async_trait;
use ractor::concurrency::{Duration as RactorDuration, JoinHandle};
use ractor::{Actor, ActorProcessingErr, ActorRef, MessagingErr};

use crate::digital_twin::{TwinMessage, ZoneReply, ZoneSpontaneousEvent};
use crate::twin_runtime::observation_streak::ObservationStreak;
use crate::vehicle_state::{FlcmContext, FlcmMessage, FlcmZoneReply, ObservationDisposition};

pub const FLCM_SILENCE_THRESHOLD: Duration = Duration::from_millis(500);

type SilenceTimer = JoinHandle<Result<(), MessagingErr<FlcmActorMsg>>>;

#[derive(Debug)]
pub struct FlcmActorVocabulary {
    pub message: FlcmMessage,
    pub turn_id: u64,
    pub tell_attempt: u32,
}

#[derive(Debug)]
pub enum FlcmActorMsg {
    Apply(FlcmActorVocabulary),
    SilenceDeadlineElapsed { deadline_id: u64 },
}

#[derive(Debug)]
pub struct FlcmActorState {
    pub ctx: FlcmContext,
    /// Test-only mute of this zone (mirrors `BcmActorState::silent`); unrelated to
    /// [`FlcmContext::silent`], which is the FLCM liveness verdict.
    ///
    /// Two `silent` flags on one actor is not a clean design: this one swallows tells
    /// in contract tests, the other is a published lamp-liveness bit. Revisit later
    /// (rename this field, or stop overloading the word).
    pub silent: bool,
    pub powered: bool,
    pub left_streak: ObservationStreak<bool>,
    pub right_streak: ObservationStreak<bool>,
    brain: ActorRef<TwinMessage>,
    silence_timer: Option<SilenceTimer>,
    deadline_id: u64,
}

impl FlcmActorState {
    pub fn new(ctx: FlcmContext, silent: bool, brain: ActorRef<TwinMessage>) -> Self {
        Self {
            ctx,
            silent,
            powered: false,
            left_streak: ObservationStreak::default(),
            right_streak: ObservationStreak::default(),
            brain,
            silence_timer: None,
            deadline_id: 0,
        }
    }
}

#[derive(Default)]
pub struct FlcmActor;

#[async_trait]
impl Actor for FlcmActor {
    type Msg = FlcmActorMsg;
    type State = FlcmActorState;
    type Arguments = FlcmActorState;

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
            FlcmActorMsg::Apply(vocab) => {
                Self::handle_apply(&myself, state, vocab)?;
            }
            FlcmActorMsg::SilenceDeadlineElapsed { deadline_id } => {
                Self::handle_silence_deadline(state, deadline_id)?;
            }
        }
        Ok(())
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        abort_silence_timer(&mut state.silence_timer);
        Ok(())
    }
}

impl FlcmActor {
    fn handle_apply(
        myself: &ActorRef<FlcmActorMsg>,
        state: &mut FlcmActorState,
        vocab: FlcmActorVocabulary,
    ) -> Result<(), ActorProcessingErr> {
        if state.silent {
            return Ok(());
        }
        let message = vocab.message;
        let reply = apply_flcm_message(state, message);
        match message {
            // Only an observed status arms the watchdog: a topology that never publishes FLCM
            // status stays `Unknown` forever instead of raising a false silence fault.
            FlcmMessage::LeftLowBeamStatusObserved(_)
            | FlcmMessage::RightLowBeamStatusObserved(_) => arm_silence_timer(myself, state),
            // Power hops only invalidate any deadline left over from the previous epoch.
            FlcmMessage::BecomeOn | FlcmMessage::BecomeOff => cancel_silence_deadline(state),
            FlcmMessage::SilenceChanged(_) => {}
        }
        state
            .brain
            .send_message(TwinMessage::ZoneReady {
                zone_id: crate::fsm::AssemblyId::Flcm,
                turn_id: vocab.turn_id,
                tell_attempt: vocab.tell_attempt,
                reply: ZoneReply::Flcm(reply),
            })
            .map_err(|e| {
                ActorProcessingErr::from(std::io::Error::other(format!(
                    "FlcmActor ZoneReady tell-back: {e:?}"
                )))
            })
    }

    fn handle_silence_deadline(
        state: &mut FlcmActorState,
        deadline_id: u64,
    ) -> Result<(), ActorProcessingErr> {
        if deadline_id != state.deadline_id || !state.powered {
            return Ok(());
        }
        state.silence_timer = None;
        let reply = state
            .ctx
            .on_receiving_message(FlcmMessage::SilenceChanged(true));
        state.ctx = reply.ctx.clone();
        state
            .brain
            .send_message(TwinMessage::ZoneSpontaneous {
                zone_id: crate::fsm::AssemblyId::Flcm,
                event: ZoneSpontaneousEvent::Flcm { reply },
            })
            .map_err(|e| {
                ActorProcessingErr::from(std::io::Error::other(format!(
                    "FlcmActor ZoneSpontaneous tell-back: {e:?}"
                )))
            })
    }
}

fn abort_silence_timer(timer: &mut Option<SilenceTimer>) {
    if let Some(handle) = timer.take() {
        handle.abort();
    }
}

fn cancel_silence_deadline(state: &mut FlcmActorState) {
    abort_silence_timer(&mut state.silence_timer);
    state.deadline_id = state.deadline_id.wrapping_add(1);
}

fn arm_silence_timer(myself: &ActorRef<FlcmActorMsg>, state: &mut FlcmActorState) {
    cancel_silence_deadline(state);
    if !state.powered {
        return;
    }
    let deadline_id = state.deadline_id;
    state.silence_timer = Some(
        myself.send_after(RactorDuration::from(FLCM_SILENCE_THRESHOLD), move || {
            FlcmActorMsg::SilenceDeadlineElapsed { deadline_id }
        }),
    );
}

fn apply_flcm_message(state: &mut FlcmActorState, message: FlcmMessage) -> FlcmZoneReply {
    let mut reply = match message {
        FlcmMessage::BecomeOn => {
            state.powered = true;
            state.left_streak = ObservationStreak::default();
            state.right_streak = ObservationStreak::default();
            state.ctx.on_receiving_message(message)
        }
        FlcmMessage::BecomeOff => {
            state.powered = false;
            state.left_streak = ObservationStreak::default();
            state.right_streak = ObservationStreak::default();
            state.ctx.on_receiving_message(message)
        }
        FlcmMessage::LeftLowBeamStatusObserved(value) => {
            let disposition = state.left_streak.observe(value);
            apply_observation(&state.ctx, message, disposition)
        }
        FlcmMessage::RightLowBeamStatusObserved(value) => {
            let disposition = state.right_streak.observe(value);
            apply_observation(&state.ctx, message, disposition)
        }
        FlcmMessage::SilenceChanged(_) => state.ctx.on_receiving_message(message),
    };

    if matches!(message, FlcmMessage::SilenceChanged(_)) {
        reply.disposition = ObservationDisposition::Lifecycle;
    }
    state.ctx = reply.ctx.clone();
    reply
}

fn apply_observation(
    ctx: &FlcmContext,
    message: FlcmMessage,
    disposition: ObservationDisposition,
) -> FlcmZoneReply {
    if matches!(disposition, ObservationDisposition::Duplicate { .. }) && !ctx.silent {
        return FlcmZoneReply {
            ctx: ctx.clone(),
            disposition,
        };
    }
    let mut reply = ctx.on_receiving_message(message);
    reply.disposition =
        if ctx.silent && matches!(disposition, ObservationDisposition::Duplicate { .. }) {
            ObservationDisposition::Lifecycle
        } else {
            disposition
        };
    reply
}

pub fn tell_flcm_zone(
    flcm: &ActorRef<FlcmActorMsg>,
    turn_id: u64,
    tell_attempt: u32,
    message: FlcmMessage,
) -> Result<(), ActorProcessingErr> {
    flcm.send_message(FlcmActorMsg::Apply(FlcmActorVocabulary {
        message,
        turn_id,
        tell_attempt,
    }))
    .map_err(|e| ActorProcessingErr::from(std::io::Error::other(format!("tell_flcm_zone: {e:?}"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digital_twin::{TwinMessage, ZoneReply};
    use crate::fsm::AssemblyId;
    use crate::vehicle_state::{FlcmContext, FlcmMessage, ObservationDisposition};
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
        let (flcm, _flcm_handle) = ractor::spawn::<FlcmActor>(FlcmActorState::new(
            FlcmContext::default(),
            false,
            brain.clone(),
        ))
        .await
        .expect("flcm actor");

        tell_flcm_zone(&flcm, 7, 1, FlcmMessage::BecomeOn).expect("tell become on");

        let TwinMessage::ZoneReady {
            zone_id,
            turn_id,
            tell_attempt,
            reply,
        } = tokio::time::timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("zone ready timeout")
            .expect("collector closed")
        else {
            panic!("expected ZoneReady tell-back");
        };
        assert_eq!(zone_id, AssemblyId::Flcm);
        assert_eq!(turn_id, 7);
        assert_eq!(tell_attempt, 1);
        match reply {
            ZoneReply::Flcm(reply) => {
                assert_eq!(reply.disposition, ObservationDisposition::Lifecycle);
            }
            other => panic!("unexpected reply {other:?}"),
        }

        tell_flcm_zone(&flcm, 8, 0, FlcmMessage::LeftLowBeamStatusObserved(true))
            .expect("tell left status");
        let TwinMessage::ZoneReady {
            zone_id, turn_id, ..
        } = tokio::time::timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("second tell-back timeout")
            .expect("collector closed")
        else {
            panic!("expected second ZoneReady tell-back");
        };
        assert_eq!(zone_id, AssemblyId::Flcm);
        assert_eq!(turn_id, 8);

        brain.stop(None);
        flcm.stop(None);
    }
}
