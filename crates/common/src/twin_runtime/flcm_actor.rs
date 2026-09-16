use std::time::{Duration, Instant};

use async_trait::async_trait;
use ractor::{Actor, ActorProcessingErr, ActorRef};

use crate::digital_twin::{TwinMessage, ZoneReply};
use crate::twin_runtime::observation_streak::ObservationStreak;
use crate::vehicle_state::{FlcmContext, FlcmMessage, FlcmZoneReply, ObservationDisposition};

pub const FLCM_SILENCE_THRESHOLD: Duration = Duration::from_millis(500);

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
}

#[derive(Debug)]
pub struct FlcmActorState {
    pub ctx: FlcmContext,
    pub silent: bool,
    pub powered: bool,
    pub last_status_at: Option<Instant>,
    pub left_streak: ObservationStreak<bool>,
    pub right_streak: ObservationStreak<bool>,
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
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        let FlcmActorMsg::Apply(vocab) = message;
        if state.silent {
            return Ok(());
        }
        let reply = apply_flcm_message(state, vocab.message, vocab.now);
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
