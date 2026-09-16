use super::observed::{ObservationDisposition, ObservedBool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SccmContext {
    /// Phase I historical boolean mirror. Observation state lives in [`Self::hazard_button`].
    pub hazard_button_on: bool,
    pub hazard_button: ObservedBool,
    pub hazard_mode: ObservedBool,
}

impl Default for SccmContext {
    fn default() -> Self {
        Self {
            hazard_button_on: false,
            hazard_button: ObservedBool::Unknown,
            hazard_mode: ObservedBool::Off,
        }
    }
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
            SccmMessage::BecomeOn => SccmZoneReply {
                ctx: Self {
                    hazard_button_on: false,
                    hazard_button: ObservedBool::Unknown,
                    hazard_mode: ObservedBool::Off,
                },
                disposition: ObservationDisposition::Lifecycle,
            },
            SccmMessage::BecomeOff => SccmZoneReply {
                ctx: Self {
                    hazard_button_on: self.hazard_button_on,
                    hazard_button: self.hazard_button,
                    hazard_mode: ObservedBool::Off,
                },
                disposition: ObservationDisposition::Lifecycle,
            },
            SccmMessage::HazardButtonObserved(value) => {
                let disposition = self.hazard_button.classify(value);
                let mut ctx = self.clone();
                let rising = self.hazard_button != ObservedBool::On && value;
                if rising {
                    ctx.hazard_mode = match ctx.hazard_mode {
                        ObservedBool::On => ObservedBool::Off,
                        _ => ObservedBool::On,
                    };
                }
                ctx.hazard_button = ObservedBool::from(value);
                ctx.hazard_button_on = value;
                SccmZoneReply { ctx, disposition }
            }
        }
    }
}
