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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BcmOutcome {
    TurnLightsChanged { left_on: bool, right_on: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BcmContext {
    pub state: BcmState,
    pub left_turn_request_on: bool,
    pub right_turn_request_on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BcmZoneReply {
    pub ctx: BcmContext,
    pub outcomes: Vec<BcmOutcome>,
}

impl BcmContext {
    pub fn on_receiving_message(&self, message: BcmMessage) -> BcmZoneReply {
        let mut ctx = self.clone();
        match message {
            BcmMessage::BecomeOn => ctx.state = BcmState::Ready,
            BcmMessage::BecomeOff => {
                ctx.state = BcmState::Off;
                ctx.left_turn_request_on = false;
                ctx.right_turn_request_on = false;
            }
            BcmMessage::HazardButtonChanged(on) if ctx.state == BcmState::Ready => {
                ctx.left_turn_request_on = on;
                ctx.right_turn_request_on = on;
            }
            BcmMessage::HazardButtonChanged(_) => {}
        }

        let changed = (ctx.left_turn_request_on, ctx.right_turn_request_on)
            != (self.left_turn_request_on, self.right_turn_request_on);
        let outcomes = changed
            .then_some(BcmOutcome::TurnLightsChanged {
                left_on: ctx.left_turn_request_on,
                right_on: ctx.right_turn_request_on,
            })
            .into_iter()
            .collect();
        BcmZoneReply { ctx, outcomes }
    }
}
