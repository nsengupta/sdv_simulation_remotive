//! L1 vehicle state: per-zone alphabet, contexts, and [`VehicleContext`].
//!
//! Each zone exposes `{Zone}State`, `{Zone}Message`, `{Zone}Outcome` where applicable.
//! **L1 handler pattern:** `{Zone}Context::on_receiving_message(msg, now) -> {Zone}ZoneReply` (headlamp
//! first). Zones import L0 only — no L2/L4.

pub mod bcm;
pub mod flcm;
pub mod front_headlamp;
pub mod health;
pub mod observed;
pub mod powertrain;
pub mod sccm;
pub mod visibility;
pub mod weather;
pub mod wiper;

pub use bcm::{BcmContext, BcmMessage, BcmOutcome, BcmState, BcmZoneReply};
pub use flcm::{FlcmContext, FlcmMessage, FlcmZoneReply};
pub use front_headlamp::{
    FrontHeadlampIncompleteCause, FrontHeadlampSwitchDirection, HeadlampContext, HeadlampMessage,
    HeadlampOutcome, HeadlampState, HeadlampZoneReply,
};
pub use health::{HealthState, VehicleHealthContext};
pub use observed::{ObservationDisposition, ObservedBool};
pub use powertrain::{
    PowertrainContext, PowertrainMessage, PowertrainMode, PowertrainOutcome, PowertrainState,
    WheelRpm,
};
pub use sccm::{SccmContext, SccmMessage, SccmZoneReply};
pub use visibility::{VisibilityContext, VisibilityMessage, VisibilityOutcome, VisibilityState};
pub use weather::WeatherContext;
pub use wiper::{WiperContext, WiperMessage, WiperOutcome, WiperState, WiperZoneReply};

/// Aggregate of all vehicle assemblies held by the digital twin.
///
/// Fields stay public for now so existing call sites keep compiling; behavior
/// lives on the per-assembly types, not here. Each assembly carries its own
/// `Default`, so the aggregate derives it.
///
/// Assembly startup/shutdown bookkeeping (formerly `remaining_assemblies`) has moved
/// into `FsmState::PreparingToStart` / `PreparingToStop`; `VehicleContext` no longer
/// carries any FSM lifecycle state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VehicleContext {
    pub sccm: SccmContext,
    pub bcm: BcmContext,
    pub flcm: FlcmContext,
    pub powertrain: PowertrainContext,
    pub health: VehicleHealthContext,
    pub visibility: VisibilityContext,
    pub weather: WeatherContext,
    pub headlamp: HeadlampContext,
    pub wiper: WiperContext,
}

impl VehicleContext {
    /// Thin delegate retained for Step 1 so existing callers stay unchanged.
    /// Inline-remove in Step 2 in favor of `health.is_healthy`.
    pub fn is_healthy(&self) -> bool {
        self.health.is_healthy()
    }
}
