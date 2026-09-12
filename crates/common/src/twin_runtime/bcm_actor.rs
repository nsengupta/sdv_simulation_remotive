use async_trait::async_trait;
use ractor::{Actor, ActorProcessingErr, ActorRef};

use crate::digital_twin::{TwinMessage, ZoneReply};
use crate::twin_runtime::observation_streak::ObservationStreak;
use crate::vehicle_state::{BcmContext, BcmMessage, BcmZoneReply, ObservationDisposition};

#[derive(Debug)]
pub struct BcmActorVocabulary {
    pub message: BcmMessage,
    pub turn_id: u64,
    pub tell_attempt: u32,
    pub brain: ActorRef<TwinMessage>,
}

#[derive(Debug)]
pub enum BcmActorMsg {
    Apply(BcmActorVocabulary),
}

#[derive(Debug)]
pub struct BcmActorState {
    pub ctx: BcmContext,
    pub silent: bool,
    pub left_streak: ObservationStreak<bool>,
    pub right_streak: ObservationStreak<bool>,
}

impl BcmActorState {
    pub fn new(ctx: BcmContext, silent: bool) -> Self {
        Self {
            ctx,
            silent,
            left_streak: ObservationStreak::default(),
            right_streak: ObservationStreak::default(),
        }
    }
}

#[derive(Default)]
pub struct BcmActor;

#[async_trait]
impl Actor for BcmActor {
    type Msg = BcmActorMsg;
    type State = BcmActorState;
    type Arguments = BcmActorState;

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
        let BcmActorMsg::Apply(vocab) = message;
        if state.silent {
            return Ok(());
        }
        let reply = apply_bcm_message(state, vocab.message);
        vocab
            .brain
            .send_message(TwinMessage::ZoneReady {
                zone_id: crate::fsm::AssemblyId::Bcm,
                turn_id: vocab.turn_id,
                tell_attempt: vocab.tell_attempt,
                reply: ZoneReply::Bcm(reply),
            })
            .map_err(|e| {
                ActorProcessingErr::from(std::io::Error::other(format!(
                    "BcmActor ZoneReady tell-back: {e:?}"
                )))
            })
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if let Some((value, duplicates)) = state.left_streak.pending_summary() {
            eprintln!("bcm.left shutdown value={value} duplicates={duplicates}");
        }
        if let Some((value, duplicates)) = state.right_streak.pending_summary() {
            eprintln!("bcm.right shutdown value={value} duplicates={duplicates}");
        }
        Ok(())
    }
}

fn apply_bcm_message(state: &mut BcmActorState, message: BcmMessage) -> BcmZoneReply {
    match message {
        BcmMessage::BecomeOn | BcmMessage::BecomeOff | BcmMessage::HazardButtonChanged(_) => {
            let mut reply = state.ctx.on_receiving_message(message);
            reply.disposition = ObservationDisposition::Lifecycle;
            state.ctx = reply.ctx.clone();
            reply
        }
        BcmMessage::LeftTurnRequestObserved(value) => observe_bcm_bool(state, message, value, true),
        BcmMessage::RightTurnRequestObserved(value) => {
            observe_bcm_bool(state, message, value, false)
        }
    }
}

fn observe_bcm_bool(
    state: &mut BcmActorState,
    message: BcmMessage,
    value: bool,
    left: bool,
) -> BcmZoneReply {
    let disposition = if left {
        state.left_streak.observe(value)
    } else {
        state.right_streak.observe(value)
    };
    match disposition {
        ObservationDisposition::Duplicate { .. } => BcmZoneReply {
            ctx: state.ctx.clone(),
            outcomes: Vec::new(),
            disposition,
        },
        ObservationDisposition::Changed {
            completed_duplicates,
        } => {
            let side = if left { "bcm.left" } else { "bcm.right" };
            eprintln!("{side} change completed_duplicates={completed_duplicates} new={value}");
            let mut reply = state.ctx.on_receiving_message(message);
            reply.disposition = disposition;
            state.ctx = reply.ctx.clone();
            reply
        }
        ObservationDisposition::Initial => {
            let mut reply = state.ctx.on_receiving_message(message);
            reply.disposition = disposition;
            state.ctx = reply.ctx.clone();
            reply
        }
        ObservationDisposition::Lifecycle => {
            let mut reply = state.ctx.on_receiving_message(message);
            reply.disposition = ObservationDisposition::Lifecycle;
            state.ctx = reply.ctx.clone();
            reply
        }
    }
}

pub fn tell_bcm_zone(
    bcm: &ActorRef<BcmActorMsg>,
    brain: &ActorRef<TwinMessage>,
    turn_id: u64,
    tell_attempt: u32,
    message: BcmMessage,
) -> Result<(), ActorProcessingErr> {
    bcm.send_message(BcmActorMsg::Apply(BcmActorVocabulary {
        message,
        turn_id,
        tell_attempt,
        brain: brain.clone(),
    }))
    .map_err(|e| ActorProcessingErr::from(std::io::Error::other(format!("tell_bcm_zone: {e:?}"))))
}
