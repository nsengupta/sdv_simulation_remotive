use async_trait::async_trait;
use ractor::{Actor, ActorProcessingErr, ActorRef};
use std::time::Instant;

use crate::digital_twin::{TwinMessage, ZoneReply};
use crate::vehicle_state::{BcmContext, BcmMessage};

#[derive(Debug)]
pub struct BcmActorVocabulary {
    pub message: BcmMessage,
    pub now: Instant,
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
}

impl BcmActorState {
    pub fn new(ctx: BcmContext, silent: bool) -> Self {
        Self { ctx, silent }
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
        let reply = state.ctx.on_receiving_message(vocab.message);
        state.ctx = reply.ctx.clone();
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
}

pub fn tell_bcm_zone(
    bcm: &ActorRef<BcmActorMsg>,
    brain: &ActorRef<TwinMessage>,
    turn_id: u64,
    tell_attempt: u32,
    message: BcmMessage,
    now: Instant,
) -> Result<(), ActorProcessingErr> {
    bcm.send_message(BcmActorMsg::Apply(BcmActorVocabulary {
        message,
        now,
        turn_id,
        tell_attempt,
        brain: brain.clone(),
    }))
    .map_err(|e| ActorProcessingErr::from(std::io::Error::other(format!("tell_bcm_zone: {e:?}"))))
}
