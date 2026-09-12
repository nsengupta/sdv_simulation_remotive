use async_trait::async_trait;
use ractor::{Actor, ActorProcessingErr, ActorRef};

use crate::digital_twin::{TwinMessage, ZoneReply};
use crate::twin_runtime::observation_streak::ObservationStreak;
use crate::vehicle_state::{ObservationDisposition, SccmContext, SccmMessage, SccmZoneReply};

#[derive(Debug)]
pub struct SccmActorVocabulary {
    pub message: SccmMessage,
    pub turn_id: u64,
    pub tell_attempt: u32,
    pub brain: ActorRef<TwinMessage>,
}

#[derive(Debug)]
pub enum SccmActorMsg {
    Apply(SccmActorVocabulary),
}

#[derive(Debug)]
pub struct SccmActorState {
    pub ctx: SccmContext,
    pub silent: bool,
    pub hazard_streak: ObservationStreak<bool>,
}

impl SccmActorState {
    pub fn new(ctx: SccmContext, silent: bool) -> Self {
        Self {
            ctx,
            silent,
            hazard_streak: ObservationStreak::default(),
        }
    }
}

#[derive(Default)]
pub struct SccmActor;

#[async_trait]
impl Actor for SccmActor {
    type Msg = SccmActorMsg;
    type State = SccmActorState;
    type Arguments = SccmActorState;

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
        let SccmActorMsg::Apply(vocab) = message;
        if state.silent {
            return Ok(());
        }
        let reply = apply_sccm_message(state, vocab.message);
        vocab
            .brain
            .send_message(TwinMessage::ZoneReady {
                zone_id: crate::fsm::AssemblyId::Sccm,
                turn_id: vocab.turn_id,
                tell_attempt: vocab.tell_attempt,
                reply: ZoneReply::Sccm(reply),
            })
            .map_err(|e| {
                ActorProcessingErr::from(std::io::Error::other(format!(
                    "SccmActor ZoneReady tell-back: {e:?}"
                )))
            })
    }

    async fn post_stop(
        &self,
        _myself: ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        if let Some((value, duplicates)) = state.hazard_streak.pending_summary() {
            eprintln!("sccm.hazard shutdown value={value} duplicates={duplicates}");
        }
        Ok(())
    }
}

fn apply_sccm_message(state: &mut SccmActorState, message: SccmMessage) -> SccmZoneReply {
    match message {
        SccmMessage::BecomeOn | SccmMessage::BecomeOff => state.ctx.on_receiving_message(message),
        SccmMessage::HazardButtonObserved(value) => {
            let disposition = state.hazard_streak.observe(value);
            match disposition {
                ObservationDisposition::Duplicate { .. } => SccmZoneReply {
                    ctx: state.ctx.clone(),
                    disposition,
                },
                ObservationDisposition::Changed {
                    completed_duplicates,
                } => {
                    eprintln!(
                        "sccm.hazard change completed_duplicates={completed_duplicates} new={value}"
                    );
                    let reply = state.ctx.on_receiving_message(message);
                    state.ctx = reply.ctx.clone();
                    SccmZoneReply {
                        ctx: reply.ctx,
                        disposition,
                    }
                }
                ObservationDisposition::Initial => {
                    let reply = state.ctx.on_receiving_message(message);
                    state.ctx = reply.ctx.clone();
                    SccmZoneReply {
                        ctx: reply.ctx,
                        disposition,
                    }
                }
                ObservationDisposition::Lifecycle => state.ctx.on_receiving_message(message),
            }
        }
    }
}

pub fn tell_sccm_zone(
    sccm: &ActorRef<SccmActorMsg>,
    brain: &ActorRef<TwinMessage>,
    turn_id: u64,
    tell_attempt: u32,
    message: SccmMessage,
) -> Result<(), ActorProcessingErr> {
    sccm.send_message(SccmActorMsg::Apply(SccmActorVocabulary {
        message,
        turn_id,
        tell_attempt,
        brain: brain.clone(),
    }))
    .map_err(|e| ActorProcessingErr::from(std::io::Error::other(format!("tell_sccm_zone: {e:?}"))))
}
