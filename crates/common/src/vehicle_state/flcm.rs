use super::observed::{ObservationDisposition, ObservedBool};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlcmContext {
    /// `On` means OK, `Off` means Fail, and `Unknown` means not observed.
    pub left_low_beam_status: ObservedBool,
    /// `On` means OK, `Off` means Fail, and `Unknown` means not observed.
    pub right_low_beam_status: ObservedBool,
    pub silent: bool,
}

/// FLCM zone vocabulary.
///
/// There is no `TimerTick` arm: liveness is owned by the actor's cancellable silence
/// deadline, which reports through [`FlcmMessage::SilenceChanged`]. A brain `TimerTick`
/// does not become a `FlcmMessage`; the parent records it as a passthrough turn (it
/// still ages headlamp ACK waits in context merge — that is not FLCM work).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlcmMessage {
    BecomeOn,
    BecomeOff,
    LeftLowBeamStatusObserved(bool),
    RightLowBeamStatusObserved(bool),
    SilenceChanged(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlcmZoneReply {
    pub ctx: FlcmContext,
    pub disposition: ObservationDisposition,
}

impl FlcmContext {
    pub fn with_silent(&self, silent: bool) -> Self {
        let mut ctx = self.clone();
        ctx.silent = silent;
        ctx
    }

    pub fn has_fault(&self) -> bool {
        self.silent
            || self.left_low_beam_status == ObservedBool::Off
            || self.right_low_beam_status == ObservedBool::Off
    }

    pub fn is_confirmed_healthy(&self) -> bool {
        !self.silent
            && self.left_low_beam_status == ObservedBool::On
            && self.right_low_beam_status == ObservedBool::On
    }

    pub fn on_receiving_message(&self, message: FlcmMessage) -> FlcmZoneReply {
        let disposition = match message {
            FlcmMessage::BecomeOn | FlcmMessage::BecomeOff | FlcmMessage::SilenceChanged(_) => {
                ObservationDisposition::Lifecycle
            }
            FlcmMessage::LeftLowBeamStatusObserved(value) => {
                self.left_low_beam_status.classify(value)
            }
            FlcmMessage::RightLowBeamStatusObserved(value) => {
                self.right_low_beam_status.classify(value)
            }
        };

        let mut ctx = self.clone();
        match message {
            FlcmMessage::BecomeOn | FlcmMessage::BecomeOff => ctx = Self::default(),
            FlcmMessage::LeftLowBeamStatusObserved(value) => {
                ctx.left_low_beam_status = ObservedBool::from(value);
                ctx.silent = false;
            }
            FlcmMessage::RightLowBeamStatusObserved(value) => {
                ctx.right_low_beam_status = ObservedBool::from(value);
                ctx.silent = false;
            }
            FlcmMessage::SilenceChanged(silent) => ctx.silent = silent,
        }

        FlcmZoneReply { ctx, disposition }
    }
}
