use std::time::{Duration, Instant};

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
    pub now: Instant,
    pub brain: ActorRef<TwinMessage>,
}

#[derive(Debug)]
pub enum FlcmActorMsg {
    Apply(FlcmActorVocabulary),
    SilenceDeadlineElapsed { deadline_id: u64 },
}

#[derive(Debug)]
pub struct FlcmActorState {
    pub ctx: FlcmContext,
    pub silent: bool,
    pub powered: bool,
    pub last_status_at: Option<Instant>,
    pub left_streak: ObservationStreak<bool>,
    pub right_streak: ObservationStreak<bool>,
    brain: Option<ActorRef<TwinMessage>>,
    silence_timer: Option<SilenceTimer>,
    deadline_id: u64,
}

impl FlcmActorState {
    pub fn new(ctx: FlcmContext, silent: bool) -> Self {
        Self {
            ctx,
            silent,
            powered: false,
            last_status_at: None,
            left_streak: ObservationStreak::default(),
            right_streak: ObservationStreak::default(),
            brain: None,
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
        state.brain = Some(vocab.brain.clone());
        let message = vocab.message;
        let reply = apply_flcm_message(state, message, vocab.now);
        match message {
            FlcmMessage::BecomeOn
            | FlcmMessage::LeftLowBeamStatusObserved(_)
            | FlcmMessage::RightLowBeamStatusObserved(_) => arm_silence_timer(myself, state),
            FlcmMessage::BecomeOff => cancel_silence_deadline(state),
            FlcmMessage::TimerTick | FlcmMessage::SilenceChanged(_) => {}
        }
        vocab
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
        let Some(brain) = state.brain.clone() else {
            return Ok(());
        };
        let reply = state
            .ctx
            .on_receiving_message(FlcmMessage::SilenceChanged(true));
        state.ctx = reply.ctx.clone();
        brain
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

fn apply_flcm_message(
    state: &mut FlcmActorState,
    message: FlcmMessage,
    now: Instant,
) -> FlcmZoneReply {
    let mut reply = match message {
        FlcmMessage::BecomeOn => {
            state.powered = true;
            state.last_status_at = Some(now);
            state.left_streak = ObservationStreak::default();
            state.right_streak = ObservationStreak::default();
            state.ctx.on_receiving_message(message)
        }
        FlcmMessage::BecomeOff => {
            state.powered = false;
            state.last_status_at = None;
            state.left_streak = ObservationStreak::default();
            state.right_streak = ObservationStreak::default();
            state.ctx.on_receiving_message(message)
        }
        FlcmMessage::LeftLowBeamStatusObserved(value) => {
            state.last_status_at = Some(now);
            let disposition = state.left_streak.observe(value);
            apply_observation(&state.ctx, message, disposition)
        }
        FlcmMessage::RightLowBeamStatusObserved(value) => {
            state.last_status_at = Some(now);
            let disposition = state.right_streak.observe(value);
            apply_observation(&state.ctx, message, disposition)
        }
        FlcmMessage::TimerTick => {
            let silence_message = message_for_timer_tick(state, now);
            state.ctx.on_receiving_message(silence_message)
        }
        FlcmMessage::SilenceChanged(_) => state.ctx.on_receiving_message(message),
    };

    if matches!(
        message,
        FlcmMessage::TimerTick | FlcmMessage::SilenceChanged(_)
    ) {
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

pub fn message_for_timer_tick(state: &FlcmActorState, now: Instant) -> FlcmMessage {
    let expired = state.powered
        && state
            .last_status_at
            .is_some_and(|last| now.saturating_duration_since(last) >= FLCM_SILENCE_THRESHOLD);
    FlcmMessage::SilenceChanged(expired)
}

pub fn tell_flcm_zone(
    flcm: &ActorRef<FlcmActorMsg>,
    brain: &ActorRef<TwinMessage>,
    turn_id: u64,
    tell_attempt: u32,
    message: FlcmMessage,
    now: Instant,
) -> Result<(), ActorProcessingErr> {
    flcm.send_message(FlcmActorMsg::Apply(FlcmActorVocabulary {
        message,
        turn_id,
        tell_attempt,
        now,
        brain: brain.clone(),
    }))
    .map_err(|e| ActorProcessingErr::from(std::io::Error::other(format!("tell_flcm_zone: {e:?}"))))
}
