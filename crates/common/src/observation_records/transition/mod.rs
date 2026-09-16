//! Transition record types — world-facing projection of FSM turns (L3).
//!
//! The pure FSM core measures time with [`std::time::Instant`] — monotonic, process-local, and
//! deliberately **not** serializable (it has no defined zero). For anything that leaves the
//! process — a file, a wire, an offline verifier — every `Instant` is projected to a
//! [`UnixTimestamp`] (wall time since [`UNIX_EPOCH`]), anchored once per session by a
//! [`SessionClock`].
//!
//! Design contract (see `docs/design-notes-runtime-observation.md`, item "(1)"):
//! - **Permanence of `Instant` inside:** [`crate::fsm::FsmState`],
//! [`crate::vehicle_state::VehicleContext`], and [`crate::fsm::RawTransitionRecord`] stay `Instant`-bearing
//! and serde-free. Nothing here mutates the functional core.
//! - **UnixTimestamp for the world:** this module owns the full, lossless mirror of those types with
//! each `Instant` replaced by a semantic wall-clock stamp since `UNIX_EPOCH`.
//!
//! Ordering for offline folding is `record_seq` (clock-independent); `recorded_at`
//! answers *how long between transitions*; `session_started_at` says *which run*.
//!
//! **Wire format:** archival codecs live in the L6 `observation` crate. These published types are
//! the live projection surface; do not embed protobuf or JSON schema derives here.

pub mod sink;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::fsm::{DomainAction, FsmEvent, FsmState, RawTransitionRecord};
use crate::vehicle_state::{
    BcmState, FrontHeadlampIncompleteCause, FrontHeadlampSwitchDirection, HeadlampContext,
    HeadlampState, ObservedBool, PowertrainContext, VehicleContext, VehicleHealthContext,
    VisibilityContext, WheelRpm, WiperState,
};

/// Wall-clock instant since the Unix Epoch, for Twin-authored observation records.
///
/// Backed by [`Duration`] so arithmetic and unit meaning stay type-safe. Storage adapters split
/// this into whole seconds plus subsecond nanoseconds; callers must not treat the inner value as
/// a bare integer without a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixTimestamp(Duration);

impl UnixTimestamp {
    /// Construct from a duration since `UNIX_EPOCH`.
    pub fn from_duration_since_epoch(value: Duration) -> Self {
        Self(value)
    }

    /// Exact duration since `UNIX_EPOCH`.
    pub fn duration_since_epoch(self) -> Duration {
        self.0
    }

    /// Whole seconds since `UNIX_EPOCH` (`Duration::as_secs`).
    pub fn unix_seconds(self) -> u64 {
        self.0.as_secs()
    }

    /// Subsecond nanoseconds (`0..=999_999_999`).
    pub fn nanosecond(self) -> u32 {
        self.0.subsec_nanos()
    }

    /// Saturating elapsed time from an earlier stamp to this one.
    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }
}

/// Per-session clock: correlates monotonic [`Instant`] with wall time since [`UNIX_EPOCH`].
///
/// Captured once at actor start. This is **not** a timestamp — it is the anchor used to
/// project monotonic instants into [`UnixTimestamp`] values. The session start itself is
/// exposed as [`Self::session_started_at`].
///
/// `started_at_instant` is the monotonic anchor; `started_at_unix` is when that anchor sits
/// on the wall clock. Any later monotonic instant `t` projects to
/// `started_at_unix + (t - started_at_instant)`.
#[derive(Debug, Clone, Copy)]
pub struct SessionClock {
    started_at_instant: Instant,
    started_at_unix: Duration,
}

impl SessionClock {
    /// Capture the (monotonic, wall) anchor pair now. Reads the wall clock exactly once.
    pub fn capture() -> Self {
        Self {
            started_at_instant: Instant::now(),
            started_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default(),
        }
    }

    /// Project a monotonic instant to a wall-clock [`UnixTimestamp`] since `UNIX_EPOCH`.
    ///
    /// `saturating_duration_since` guards the (not-expected) case of an instant before the
    /// anchor, yielding the anchor's own wall stamp rather than underflowing.
    pub fn project(&self, t: &Instant) -> UnixTimestamp {
        UnixTimestamp::from_duration_since_epoch(
            self.started_at_unix + t.saturating_duration_since(self.started_at_instant),
        )
    }

    /// When this session started on the wall clock.
    pub fn session_started_at(&self) -> UnixTimestamp {
        UnixTimestamp::from_duration_since_epoch(self.started_at_unix)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishedHeadlampState {
    Off,
    Ready,
    OnRequested,
    On,
    OffRequested,
}

impl From<&HeadlampState> for PublishedHeadlampState {
    fn from(s: &HeadlampState) -> Self {
        match s {
            HeadlampState::Off => Self::Off,
            HeadlampState::Ready => Self::Ready,
            HeadlampState::OnRequested => Self::OnRequested,
            HeadlampState::On => Self::On,
            HeadlampState::OffRequested => Self::OffRequested,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishedFrontHeadlampSwitchDirection {
    On,
    Off,
}

impl From<&FrontHeadlampSwitchDirection> for PublishedFrontHeadlampSwitchDirection {
    fn from(d: &FrontHeadlampSwitchDirection) -> Self {
        match d {
            FrontHeadlampSwitchDirection::On => Self::On,
            FrontHeadlampSwitchDirection::Off => Self::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishedFrontHeadlampIncompleteCause {
    TimedOut,
    NegativeAck,
}

impl From<&FrontHeadlampIncompleteCause> for PublishedFrontHeadlampIncompleteCause {
    fn from(c: &FrontHeadlampIncompleteCause) -> Self {
        match c {
            FrontHeadlampIncompleteCause::TimedOut => Self::TimedOut,
            FrontHeadlampIncompleteCause::NegativeAck => Self::NegativeAck,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishedOperational {
    LightingUnsafe,
}

impl From<&crate::fsm::Operational> for PublishedOperational {
    fn from(op: &crate::fsm::Operational) -> Self {
        match op {
            crate::fsm::Operational::LightingUnsafe => Self::LightingUnsafe,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishedObservedBool {
    #[default]
    Unknown,
    Off,
    On,
}

impl From<ObservedBool> for PublishedObservedBool {
    fn from(value: ObservedBool) -> Self {
        match value {
            ObservedBool::Unknown => Self::Unknown,
            ObservedBool::Off => Self::Off,
            ObservedBool::On => Self::On,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishedFsmEvent {
    PowerOn,
    PowerOff,
    UpdateRpm(u16),
    HazardButtonChanged(bool),
    HazardButtonObserved(bool),
    LeftTurnRequestObserved(bool),
    RightTurnRequestObserved(bool),
    /// FLCM low-beam status observation. `true` = Ok, `false` = Fail (DBC polarity is
    /// inverted in the bridge decoder, not here).
    LeftLowBeamStatusObserved(bool),
    RightLowBeamStatusObserved(bool),
    UpdateAmbientLux(u16),
    FrontHeadlampOnAck,
    FrontHeadlampOffAck,
    FrontHeadlampActuationIncomplete {
        direction: PublishedFrontHeadlampSwitchDirection,
        cause: PublishedFrontHeadlampIncompleteCause,
    },
    TimerTick,
    Internal(PublishedOperational),
    RainsStarted,
    RainsStopped,
}

impl From<&FsmEvent> for PublishedFsmEvent {
    fn from(e: &FsmEvent) -> Self {
        match e {
            FsmEvent::PowerOn => Self::PowerOn,
            FsmEvent::PowerOff => Self::PowerOff,
            FsmEvent::UpdateRpm(rpm) => Self::UpdateRpm(*rpm),
            FsmEvent::HazardButtonChanged(pressed) => Self::HazardButtonChanged(*pressed),
            FsmEvent::HazardButtonObserved(pressed) => Self::HazardButtonObserved(*pressed),
            FsmEvent::LeftTurnRequestObserved(pressed) => Self::LeftTurnRequestObserved(*pressed),
            FsmEvent::RightTurnRequestObserved(pressed) => Self::RightTurnRequestObserved(*pressed),
            FsmEvent::LeftLowBeamStatusObserved(ok) => Self::LeftLowBeamStatusObserved(*ok),
            FsmEvent::RightLowBeamStatusObserved(ok) => Self::RightLowBeamStatusObserved(*ok),
            FsmEvent::UpdateAmbientLux(lux) => Self::UpdateAmbientLux(*lux),
            FsmEvent::FrontHeadlampOnAck => Self::FrontHeadlampOnAck,
            FsmEvent::FrontHeadlampOffAck => Self::FrontHeadlampOffAck,
            FsmEvent::FrontHeadlampActuationIncomplete { direction, cause } => {
                Self::FrontHeadlampActuationIncomplete {
                    direction: direction.into(),
                    cause: cause.into(),
                }
            }
            FsmEvent::TimerTick => Self::TimerTick,
            FsmEvent::Internal(op) => Self::Internal(op.into()),
            FsmEvent::RainsStarted => Self::RainsStarted,
            FsmEvent::RainsStopped => Self::RainsStopped,
            // AssemblyZoneReady remains unpublished; map to TimerTick as a neutral placeholder.
            // The FLCM silence verdict rides this hop, so its ledger row is identified by
            // `flcm.silent` in the published context rather than by the event label.
            FsmEvent::AssemblyZoneReady(_) => Self::TimerTick,
        }
    }
}

/// World-facing domain intents. Mirrors [`DomainAction`] **minus** runtime control hints
/// (`StartAssemblies`, `StopAssemblies`), which are already excluded from the
/// recorded action list (WI-1) and carry no ledger-level meaning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishedDomainAction {
    StartBuzzer,
    StopBuzzer,
    PublishStateSync,
    LogWarning(String),
    RequestFrontHeadlampOn,
    RequestFrontHeadlampOff,
    RequestWiperStart,
    RequestWiperStop,
    SetTurnLights { left_on: bool, right_on: bool },
}

impl PublishedDomainAction {
    /// Project a domain action, dropping internal coordination hints.
    fn project(action: &DomainAction) -> Option<Self> {
        match action {
            DomainAction::StartBuzzer => Some(Self::StartBuzzer),
            DomainAction::StopBuzzer => Some(Self::StopBuzzer),
            DomainAction::PublishStateSync => Some(Self::PublishStateSync),
            DomainAction::LogWarning(msg) => Some(Self::LogWarning(msg.clone())),
            DomainAction::RequestFrontHeadlampOn => Some(Self::RequestFrontHeadlampOn),
            DomainAction::RequestFrontHeadlampOff => Some(Self::RequestFrontHeadlampOff),
            DomainAction::RequestWiperStart => Some(Self::RequestWiperStart),
            DomainAction::RequestWiperStop => Some(Self::RequestWiperStop),
            DomainAction::SetTurnLights { left_on, right_on } => Some(Self::SetTurnLights {
                left_on: *left_on,
                right_on: *right_on,
            }),
            // Internal coordination signals: not domain intents, not ledger-visible.
            DomainAction::StartAssemblies(_) | DomainAction::StopAssemblies(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishedFsmState {
    Off,
    PreparingToStart,
    Idle,
    Driving,
    DrivingDangerously,
    /// When the warning state was entered, projected to wall clock.
    ExtremeOperationWarning {
        entered_at: UnixTimestamp,
    },
    PreparingToStop,
}

impl PublishedFsmState {
    fn project(state: &FsmState, clock: &SessionClock) -> Self {
        match state {
            FsmState::Off => Self::Off,
            FsmState::PreparingToStart { .. } => Self::PreparingToStart,
            FsmState::Idle => Self::Idle,
            FsmState::Driving => Self::Driving,
            FsmState::DrivingDangerously => Self::DrivingDangerously,
            FsmState::ExtremeOperationWarning(at) => Self::ExtremeOperationWarning {
                entered_at: clock.project(at),
            },
            FsmState::PreparingToStop { .. } => Self::PreparingToStop,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedWheelRpm {
    pub front_left: u16,
    pub front_right: u16,
    pub rear_left: u16,
    pub rear_right: u16,
}

impl From<&WheelRpm> for PublishedWheelRpm {
    fn from(w: &WheelRpm) -> Self {
        Self {
            front_left: w.front_left,
            front_right: w.front_right,
            rear_left: w.rear_left,
            rear_right: w.rear_right,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedPowertrainContext {
    pub wheel_rpm: PublishedWheelRpm,
    pub speed_kph: u16,
}

impl From<&PowertrainContext> for PublishedPowertrainContext {
    fn from(p: &PowertrainContext) -> Self {
        Self {
            wheel_rpm: (&p.wheel_rpm).into(),
            speed_kph: p.speed_kph,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedHealthContext {
    pub fuel_level_pct: u8,
    pub oil_pressure_kpa: u8,
    pub tyre_pressure_ok: bool,
}

impl From<&VehicleHealthContext> for PublishedHealthContext {
    fn from(h: &VehicleHealthContext) -> Self {
        Self {
            fuel_level_pct: h.fuel_level_pct,
            oil_pressure_kpa: h.oil_pressure_kpa,
            tyre_pressure_ok: h.tyre_pressure_ok,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedVisibilityContext {
    pub ambient_lux: u16,
}

impl From<&VisibilityContext> for PublishedVisibilityContext {
    fn from(v: &VisibilityContext) -> Self {
        Self {
            ambient_lux: v.ambient_lux,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedWeatherContext {
    pub raining: bool,
}

impl From<&crate::vehicle_state::WeatherContext> for PublishedWeatherContext {
    fn from(w: &crate::vehicle_state::WeatherContext) -> Self {
        Self { raining: w.raining }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishedWiperState {
    Off,
    Ready,
    Running,
}

impl From<&WiperState> for PublishedWiperState {
    fn from(s: &WiperState) -> Self {
        match s {
            WiperState::Off => Self::Off,
            WiperState::Ready => Self::Ready,
            WiperState::Running => Self::Running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedWiperContext {
    pub state: PublishedWiperState,
}

impl From<&crate::vehicle_state::WiperContext> for PublishedWiperContext {
    fn from(w: &crate::vehicle_state::WiperContext) -> Self {
        Self {
            state: (&w.state).into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedSccmContext {
    pub hazard_button_on: PublishedObservedBool,
    pub hazard_mode_on: PublishedObservedBool,
}

impl From<&crate::vehicle_state::SccmContext> for PublishedSccmContext {
    fn from(s: &crate::vehicle_state::SccmContext) -> Self {
        Self {
            hazard_button_on: s.hazard_button.into(),
            hazard_mode_on: s.hazard_mode.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishedBcmState {
    Off,
    Ready,
}

impl From<&BcmState> for PublishedBcmState {
    fn from(s: &BcmState) -> Self {
        match s {
            BcmState::Off => Self::Off,
            BcmState::Ready => Self::Ready,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedBcmContext {
    pub state: PublishedBcmState,
    pub left_turn_request_on: PublishedObservedBool,
    pub right_turn_request_on: PublishedObservedBool,
}

impl From<&crate::vehicle_state::BcmContext> for PublishedBcmContext {
    fn from(b: &crate::vehicle_state::BcmContext) -> Self {
        Self {
            state: (&b.state).into(),
            left_turn_request_on: b.left_turn_request.into(),
            right_turn_request_on: b.right_turn_request.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedFlcmContext {
    pub left_low_beam_status_ok: PublishedObservedBool,
    pub right_low_beam_status_ok: PublishedObservedBool,
    pub silent: bool,
}

impl From<&crate::vehicle_state::FlcmContext> for PublishedFlcmContext {
    fn from(flcm: &crate::vehicle_state::FlcmContext) -> Self {
        Self {
            left_low_beam_status_ok: flcm.left_low_beam_status.into(),
            right_low_beam_status_ok: flcm.right_low_beam_status.into(),
            silent: flcm.silent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedHeadlampContext {
    pub state: PublishedHeadlampState,
    /// When ACK wait began, projected to wall clock (`None` if not pending).
    pub ack_pending_since: Option<UnixTimestamp>,
}

impl PublishedHeadlampContext {
    fn project(h: &HeadlampContext, clock: &SessionClock) -> Self {
        Self {
            state: (&h.state).into(),
            ack_pending_since: h.ack_pending_since.as_ref().map(|t| clock.project(t)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedVehicleContext {
    pub sccm: PublishedSccmContext,
    pub bcm: PublishedBcmContext,
    pub flcm: PublishedFlcmContext,
    pub powertrain: PublishedPowertrainContext,
    pub health: PublishedHealthContext,
    pub visibility: PublishedVisibilityContext,
    pub weather: PublishedWeatherContext,
    pub headlamp: PublishedHeadlampContext,
    pub wiper: PublishedWiperContext,
}

impl PublishedVehicleContext {
    pub(crate) fn project(ctx: &VehicleContext, clock: &SessionClock) -> Self {
        Self {
            sccm: (&ctx.sccm).into(),
            bcm: (&ctx.bcm).into(),
            flcm: (&ctx.flcm).into(),
            powertrain: (&ctx.powertrain).into(),
            health: (&ctx.health).into(),
            visibility: (&ctx.visibility).into(),
            weather: (&ctx.weather).into(),
            headlamp: PublishedHeadlampContext::project(&ctx.headlamp, clock),
            wiper: (&ctx.wiper).into(),
        }
    }
}

/// The `Instant`-free transition record emitted "to the world".
#[derive(Debug, Clone, PartialEq)]
pub struct PublishedTransitionRecord {
    pub car_identity: String,
    /// Which run produced this record (Twin session start on the wall clock).
    pub session_started_at: UnixTimestamp,
    /// Monotonic, clock-independent ledger order (Counter A).
    pub record_seq: u64,
    /// When this transition was recorded on the wall clock.
    pub recorded_at: UnixTimestamp,
    pub event: PublishedFsmEvent,
    pub old_state: PublishedFsmState,
    pub next_state: PublishedFsmState,
    pub old_ctx: PublishedVehicleContext,
    pub current_ctx: PublishedVehicleContext,
    pub actions: Vec<PublishedDomainAction>,
}

impl PublishedTransitionRecord {
    /// Project a pure [`RawTransitionRecord`] into its wall-clock-stamped form.
    ///
    /// Composition root: packages envelope metadata and delegates each field to its published
    /// type's projection (`From` for instant-free fields, `project` where a [`SessionClock`]
    /// is required). Emitted via [`sink::TransitionRecordSink`] (e.g. tokio mpsc); wire codecs
    /// map from this type separately.
    pub fn project(
        raw: &RawTransitionRecord,
        car_identity: &str,
        record_seq: u64,
        clock: &SessionClock,
    ) -> Self {
        Self {
            car_identity: car_identity.to_owned(),
            session_started_at: clock.session_started_at(),
            record_seq,
            recorded_at: clock.project(&raw.at),
            event: (&raw.event).into(),
            old_state: PublishedFsmState::project(&raw.old_state, clock),
            next_state: PublishedFsmState::project(&raw.next_state, clock),
            old_ctx: PublishedVehicleContext::project(&raw.old_ctx, clock),
            current_ctx: PublishedVehicleContext::project(&raw.current_ctx, clock),
            actions: raw
                .actions
                .iter()
                .filter_map(PublishedDomainAction::project)
                .collect(),
        }
    }
}

#[cfg(test)]
mod unix_timestamp_tests {
    use super::*;

    #[test]
    fn unix_timestamp_splits_and_reconstructs_exactly() {
        let value =
            UnixTimestamp::from_duration_since_epoch(Duration::new(1_752_724_801, 120_000_000));
        assert_eq!(value.unix_seconds(), 1_752_724_801);
        assert_eq!(value.nanosecond(), 120_000_000);
        assert_eq!(
            value.duration_since_epoch(),
            Duration::new(1_752_724_801, 120_000_000)
        );
        assert_eq!(
            UnixTimestamp::from_duration_since_epoch(Duration::new(
                value.unix_seconds(),
                value.nanosecond(),
            )),
            value
        );
    }

    #[test]
    fn unix_timestamp_orders_across_a_second_boundary() {
        let earlier = UnixTimestamp::from_duration_since_epoch(Duration::new(100, 999_999_999));
        let later = UnixTimestamp::from_duration_since_epoch(Duration::new(101, 0));
        assert!(earlier < later);
        assert_eq!(
            later.saturating_duration_since(earlier),
            Duration::from_nanos(1)
        );
    }
}

#[cfg(test)]
mod published_projection_tests {
    use super::*;
    use crate::vehicle_state::{ObservedBool, VehicleContext, WiperState};

    #[test]
    fn published_fsm_event_keeps_rain_variants() {
        assert_eq!(
            PublishedFsmEvent::from(&FsmEvent::RainsStarted),
            PublishedFsmEvent::RainsStarted
        );
        assert_eq!(
            PublishedFsmEvent::from(&FsmEvent::RainsStopped),
            PublishedFsmEvent::RainsStopped
        );
    }

    #[test]
    fn published_vehicle_context_includes_weather_wiper_and_flcm() {
        let clock = SessionClock::capture();
        let mut ctx = VehicleContext::default();
        ctx.weather.raining = true;
        ctx.wiper.state = WiperState::Running;
        ctx.flcm.left_low_beam_status = ObservedBool::On;
        ctx.flcm.right_low_beam_status = ObservedBool::Off;
        ctx.flcm.silent = true;
        let pub_ctx = PublishedVehicleContext::project(&ctx, &clock);
        assert!(pub_ctx.weather.raining);
        assert_eq!(pub_ctx.wiper.state, PublishedWiperState::Running);
        assert_eq!(
            pub_ctx.flcm.left_low_beam_status_ok,
            PublishedObservedBool::On
        );
        assert_eq!(
            pub_ctx.flcm.right_low_beam_status_ok,
            PublishedObservedBool::Off
        );
        assert!(pub_ctx.flcm.silent);
    }
}
