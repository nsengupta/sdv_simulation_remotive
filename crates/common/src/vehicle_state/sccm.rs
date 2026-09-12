use super::observed::{ObservationDisposition, ObservedBool};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SccmContext {
    /// Phase I historical boolean mirror. Observation state lives in [`Self::hazard_button`].
    pub hazard_button_on: bool,
    pub hazard_button: ObservedBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SccmMessage {
    BecomeOn,
    BecomeOff,
    HazardButtonObserved(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SccmZoneReply {
    pub ctx: SccmContext,
    pub disposition: ObservationDisposition,
}

impl SccmContext {
    pub fn on_receiving_message(&self, message: SccmMessage) -> SccmZoneReply {
        match message {
            SccmMessage::BecomeOn | SccmMessage::BecomeOff => SccmZoneReply {
                ctx: self.clone(),
                disposition: ObservationDisposition::Lifecycle,
            },
            SccmMessage::HazardButtonObserved(value) => {
                let disposition = self.hazard_button.classify(value);
                let mut ctx = self.clone();
                ctx.hazard_button = ObservedBool::from(value);
                ctx.hazard_button_on = value;
                SccmZoneReply { ctx, disposition }
            }
        }
    }
}
