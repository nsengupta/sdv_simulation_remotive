use super::observed::{ObservationDisposition, ObservedBool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BcmState {
    #[default]
    Off,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BcmMessage {
    BecomeOn,
    BecomeOff,
    HazardButtonChanged(bool),
    LeftTurnRequestObserved(bool),
    RightTurnRequestObserved(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BcmOutcome {
    TurnLightsChanged { left_on: bool, right_on: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BcmContext {
    pub state: BcmState,
    /// Phase I historical boolean mirror. Observation state lives in the `ObservedBool` fields.
    pub left_turn_request_on: bool,
    pub right_turn_request_on: bool,
    pub left_turn_request: ObservedBool,
    pub right_turn_request: ObservedBool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BcmZoneReply {
    pub ctx: BcmContext,
    pub outcomes: Vec<BcmOutcome>,
    pub disposition: ObservationDisposition,
}

impl BcmContext {
    pub fn observation_disposition(&self, message: BcmMessage) -> ObservationDisposition {
        match message {
            BcmMessage::BecomeOn | BcmMessage::BecomeOff => ObservationDisposition::Lifecycle,
            BcmMessage::LeftTurnRequestObserved(value) => self.left_turn_request.classify(value),
            BcmMessage::RightTurnRequestObserved(value) => self.right_turn_request.classify(value),
            BcmMessage::HazardButtonChanged(_) => ObservationDisposition::Lifecycle,
        }
    }

    pub fn on_receiving_message(&self, message: BcmMessage) -> BcmZoneReply {
        let disposition = self.observation_disposition(message);
        let mut ctx = self.clone();
        match message {
            BcmMessage::BecomeOn => ctx.state = BcmState::Ready,
            BcmMessage::BecomeOff => {
                ctx.state = BcmState::Off;
            }
            BcmMessage::HazardButtonChanged(_) => {}
            BcmMessage::LeftTurnRequestObserved(value) => {
                ctx.left_turn_request = ObservedBool::from(value);
                ctx.left_turn_request_on = value;
            }
            BcmMessage::RightTurnRequestObserved(value) => {
                ctx.right_turn_request = ObservedBool::from(value);
                ctx.right_turn_request_on = value;
            }
        }

        BcmZoneReply {
            ctx,
            outcomes: Vec::new(),
            disposition,
        }
    }
}
