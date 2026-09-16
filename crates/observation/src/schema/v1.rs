//! Schema version 1: validated identifiers, timestamps, manifest, stream envelopes, and the
//! explicit archival DTOs projected from live `common::facade` records.
//!
//! Every DTO here is a deliberate mirror of a live record, never a direct serialization of it
//! (see `docs/DESIGN.md`). Projection
//! functions map every live enum variant explicitly; there are no wildcard arms, so a future
//! live variant fails to compile here rather than being silently dropped or misfiled.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};
use time::{OffsetDateTime, UtcOffset};
use uuid::Uuid;

use common::facade::{
    DiagnosticKind, DiagnosticLevel, DiagnosticRecord, PublishedBcmContext, PublishedBcmState,
    PublishedDomainAction, PublishedFrontHeadlampIncompleteCause,
    PublishedFrontHeadlampSwitchDirection, PublishedFsmEvent, PublishedFsmState,
    PublishedHeadlampContext, PublishedHeadlampState, PublishedHealthContext,
    PublishedObservedBool, PublishedOperational, PublishedPowertrainContext, PublishedSccmContext,
    PublishedTransitionRecord, PublishedVehicleContext, PublishedVisibilityContext,
    PublishedWeatherContext, PublishedWheelRpm, PublishedWiperContext, PublishedWiperState,
    UnixTimestamp,
};
use common::fsm::FrontHeadlampIncompleteCause;

use crate::ObservationError;
use crate::schema::CURRENT_SCHEMA_VERSION;

const TIMESTAMP_DISPLAY_FORMAT: &[time::format_description::BorrowedFormatItem<'_>] = time::macros::format_description!(
    "[year]-[month]-[day] | [hour]:[minute]:[second]:[subsecond digits:9] (UTC)"
);

const MAX_NANOSECOND: u32 = 999_999_999;

/// A validated run identifier — a UUID, canonically v4 in production runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(Uuid);

impl RunId {
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Portable schema-v1 wall-clock stamp: whole Unix seconds plus subsecond nanoseconds.
///
/// This is the archival form of [`UnixTimestamp`]. `unix_seconds` is never milliseconds.
/// `nanosecond` is always `0..=999_999_999`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct UnixTimestampV1 {
    pub unix_seconds: u64,
    pub nanosecond: u32,
}

impl UnixTimestampV1 {
    pub fn new(unix_seconds: u64, nanosecond: u32) -> Result<Self, ObservationError> {
        if nanosecond > MAX_NANOSECOND {
            return Err(ObservationError::InvalidTimestamp {
                value: format!("{{unix_seconds:{unix_seconds},nanosecond:{nanosecond}}}"),
                reason: format!("nanosecond must be <= {MAX_NANOSECOND}"),
            });
        }
        Ok(Self {
            unix_seconds,
            nanosecond,
        })
    }

    pub fn from_live(value: UnixTimestamp) -> Self {
        Self {
            unix_seconds: value.unix_seconds(),
            nanosecond: value.nanosecond(),
        }
    }

    pub fn to_live(self) -> UnixTimestamp {
        UnixTimestamp::from_duration_since_epoch(Duration::new(self.unix_seconds, self.nanosecond))
    }

    pub fn from_duration_since_epoch(value: Duration) -> Self {
        Self::from_live(UnixTimestamp::from_duration_since_epoch(value))
    }

    /// Presentation-only UTC wall time: `yyyy-mm-dd | HH:mm:ss:nnnnnnnnn (UTC)`.
    pub fn to_display_utc(self) -> Result<String, ObservationError> {
        let nanos = i128::from(self.unix_seconds) * 1_000_000_000 + i128::from(self.nanosecond);
        let parsed = OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|source| {
            ObservationError::InvalidTimestamp {
                value: format!(
                    "{{unix_seconds:{},nanosecond:{}}}",
                    self.unix_seconds, self.nanosecond
                ),
                reason: source.to_string(),
            }
        })?;
        parsed
            .to_offset(UtcOffset::UTC)
            .format(TIMESTAMP_DISPLAY_FORMAT)
            .map_err(|source| ObservationError::InvalidTimestamp {
                value: format!(
                    "{{unix_seconds:{},nanosecond:{}}}",
                    self.unix_seconds, self.nanosecond
                ),
                reason: source.to_string(),
            })
    }
}

impl fmt::Display for UnixTimestampV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_display_utc() {
            Ok(text) => f.write_str(&text),
            Err(_) => write!(
                f,
                "{{unix_seconds:{},nanosecond:{}}}",
                self.unix_seconds, self.nanosecond
            ),
        }
    }
}

impl<'de> Deserialize<'de> for UnixTimestampV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            unix_seconds: u64,
            nanosecond: u32,
        }
        let raw = Raw::deserialize(deserializer)?;
        UnixTimestampV1::new(raw.unix_seconds, raw.nanosecond).map_err(serde::de::Error::custom)
    }
}

/// Compatibility alias used by older call sites; schema storage is [`UnixTimestampV1`].
pub type Timestamp = UnixTimestampV1;

/// Run-level metadata common to the manifest and every stream envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunMetadata {
    pub run_id: RunId,
    pub created_at: UnixTimestampV1,
    pub session_started_at: UnixTimestampV1,
    pub vehicle_identity: String,
    pub scenario: Option<ScenarioMetadata>,
}

impl RunMetadata {
    pub fn new(
        run_id: RunId,
        created_at: UnixTimestampV1,
        session_started_at: UnixTimestampV1,
        vehicle_identity: impl Into<String>,
        scenario: Option<ScenarioMetadata>,
    ) -> Self {
        Self {
            run_id,
            created_at,
            session_started_at,
            vehicle_identity: vehicle_identity.into(),
            scenario,
        }
    }

    pub fn now(
        run_id: RunId,
        session_started_at: UnixTimestampV1,
        vehicle_identity: impl Into<String>,
        scenario: Option<ScenarioMetadata>,
    ) -> Result<Self, ObservationError> {
        let now = OffsetDateTime::now_utc();
        let unix_seconds = u64::try_from(now.unix_timestamp()).map_err(|source| {
            ObservationError::InvalidTimestamp {
                value: "now".into(),
                reason: source.to_string(),
            }
        })?;
        let created_at = UnixTimestampV1::new(unix_seconds, now.nanosecond())?;
        Ok(Self::new(
            run_id,
            created_at,
            session_started_at,
            vehicle_identity,
            scenario,
        ))
    }
}

/// Optional, deliberately narrow scenario provenance. File capture always writes `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioMetadata {
    pub name: String,
    pub source: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestV1 {
    pub schema_version: u32,
    pub run_id: RunId,
    pub created_at: UnixTimestampV1,
    pub session_started_at: UnixTimestampV1,
    pub vehicle: VehicleV1,
    pub scenario: Option<ScenarioMetadata>,
    pub streams: StreamsV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VehicleV1 {
    pub identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamsV1 {
    pub diagnostic: String,
    pub ledger: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamEnvelopeV1<T> {
    pub schema_version: u32,
    pub run_id: RunId,
    pub vehicle_identity: String,
    pub recorded_at: UnixTimestampV1,
    pub payload: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FsmEventV1 {
    PowerOn,
    PowerOff,
    UpdateRpm {
        rpm: u16,
    },
    HazardButtonChanged {
        pressed: bool,
    },
    HazardButtonObserved {
        pressed: bool,
    },
    LeftTurnRequestObserved {
        pressed: bool,
    },
    RightTurnRequestObserved {
        pressed: bool,
    },
    UpdateAmbientLux {
        lux: u16,
    },
    FrontHeadlampOnAck,
    FrontHeadlampOffAck,
    FrontHeadlampActuationIncomplete {
        direction: FrontHeadlampSwitchDirectionV1,
        cause: FrontHeadlampIncompleteCauseV1,
    },
    TimerTick,
    Internal {
        operational: OperationalV1,
    },
    RainsStarted,
    RainsStopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FsmStateV1 {
    Off,
    PreparingToStart,
    Idle,
    Driving,
    DrivingDangerously,
    ExtremeOperationWarning { entered_at: UnixTimestampV1 },
    PreparingToStop,
}

impl FsmStateV1 {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::PreparingToStart => "PreparingToStart",
            Self::Idle => "Idle",
            Self::Driving => "Driving",
            Self::DrivingDangerously => "DrivingDangerously",
            Self::ExtremeOperationWarning { .. } => "ExtremeOperationWarning",
            Self::PreparingToStop => "PreparingToStop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DomainActionV1 {
    StartBuzzer,
    StopBuzzer,
    PublishStateSync,
    LogWarning { message: String },
    RequestFrontHeadlampOn,
    RequestFrontHeadlampOff,
    RequestWiperStart,
    RequestWiperStop,
    SetTurnLights { left_on: bool, right_on: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevelV1 {
    Info,
    Action,
    Alert,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiagnosticKindV1 {
    Text {
        text: String,
    },
    Boot,
    TimerTick,
    HeadlampActuationUnconfirmed {
        on: bool,
        cause: FrontHeadlampIncompleteCauseV1,
    },
    RainChanged {
        raining: bool,
    },
    WiperMotionChanged {
        wiping: bool,
    },
    ActuationFailure {
        action: String,
        error: String,
    },
    TransitionSinkFull,
    TransitionSinkClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticPayloadV1 {
    pub level: DiagnosticLevelV1,
    pub source: String,
    pub kind: DiagnosticKindV1,
    pub session_started_at: UnixTimestampV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerPayloadV1 {
    pub session_started_at: UnixTimestampV1,
    pub record_seq: u64,
    pub event: FsmEventV1,
    pub old_state: FsmStateV1,
    pub next_state: FsmStateV1,
    pub old_ctx: VehicleContextV1,
    pub current_ctx: VehicleContextV1,
    pub actions: Vec<DomainActionV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WheelRpmV1 {
    pub front_left: u16,
    pub front_right: u16,
    pub rear_left: u16,
    pub rear_right: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowertrainContextV1 {
    pub wheel_rpm: WheelRpmV1,
    pub speed_kph: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthContextV1 {
    pub fuel_level_pct: u8,
    pub oil_pressure_kpa: u8,
    pub tyre_pressure_ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityContextV1 {
    pub ambient_lux: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherContextV1 {
    pub raining: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WiperStateV1 {
    Off,
    Ready,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WiperContextV1 {
    pub state: WiperStateV1,
}

/// Tri-state observed boolean written by schema v5.
///
/// Historical v1–v4 JSON booleans deserialize as [`Off`](Self::Off) / [`On`](Self::On).
/// Missing fields default to [`Off`](Self::Off) to preserve the previous `false` default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedBoolV1 {
    Unknown,
    Off,
    On,
}

impl Default for ObservedBoolV1 {
    fn default() -> Self {
        Self::Off
    }
}

impl From<PublishedObservedBool> for ObservedBoolV1 {
    fn from(value: PublishedObservedBool) -> Self {
        match value {
            PublishedObservedBool::Unknown => Self::Unknown,
            PublishedObservedBool::Off => Self::Off,
            PublishedObservedBool::On => Self::On,
        }
    }
}

impl From<ObservedBoolV1> for PublishedObservedBool {
    fn from(value: ObservedBoolV1) -> Self {
        match value {
            ObservedBoolV1::Unknown => Self::Unknown,
            ObservedBoolV1::Off => Self::Off,
            ObservedBoolV1::On => Self::On,
        }
    }
}

impl<'de> Deserialize<'de> for ObservedBoolV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ObservedBoolVisitor;

        impl<'de> serde::de::Visitor<'de> for ObservedBoolVisitor {
            type Value = ObservedBoolV1;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an observed bool (unknown/off/on) or a historical boolean")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(if value {
                    ObservedBoolV1::On
                } else {
                    ObservedBoolV1::Off
                })
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "unknown" => Ok(ObservedBoolV1::Unknown),
                    "off" => Ok(ObservedBoolV1::Off),
                    "on" => Ok(ObservedBoolV1::On),
                    other => Err(E::unknown_variant(other, &["unknown", "off", "on"])),
                }
            }
        }

        deserializer.deserialize_any(ObservedBoolVisitor)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SccmContextV1 {
    pub hazard_button_on: ObservedBoolV1,
    #[serde(default)]
    pub hazard_mode_on: ObservedBoolV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BcmStateV1 {
    Off,
    Ready,
}

impl Default for BcmStateV1 {
    fn default() -> Self {
        Self::Off
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BcmContextV1 {
    pub state: BcmStateV1,
    pub left_turn_request_on: ObservedBoolV1,
    pub right_turn_request_on: ObservedBoolV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadlampStateV1 {
    Off,
    Ready,
    OnRequested,
    On,
    OffRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadlampContextV1 {
    pub state: HeadlampStateV1,
    pub ack_pending_since: Option<UnixTimestampV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VehicleContextV1 {
    #[serde(default)]
    pub sccm: SccmContextV1,
    #[serde(default)]
    pub bcm: BcmContextV1,
    pub powertrain: PowertrainContextV1,
    pub health: HealthContextV1,
    pub visibility: VisibilityContextV1,
    pub weather: WeatherContextV1,
    pub headlamp: HeadlampContextV1,
    pub wiper: WiperContextV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontHeadlampSwitchDirectionV1 {
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontHeadlampIncompleteCauseV1 {
    TimedOut,
    NegativeAck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationalV1 {
    LightingUnsafe,
}

/// Project a live diagnostic record into its archival DTO envelope.
pub fn diagnostic_envelope(
    metadata: &RunMetadata,
    record: &DiagnosticRecord,
) -> Result<StreamEnvelopeV1<DiagnosticPayloadV1>, ObservationError> {
    let session_started_at = UnixTimestampV1::from_live(record.session_started_at);
    if session_started_at != metadata.session_started_at {
        return Err(ObservationError::SessionMismatch {
            expected: metadata.session_started_at.to_string(),
            found: session_started_at.to_string(),
        });
    }
    let payload = DiagnosticPayloadV1 {
        level: project_diagnostic_level(record.level),
        source: record.source.to_string(),
        kind: project_diagnostic_kind(&record.kind),
        session_started_at,
    };
    Ok(StreamEnvelopeV1 {
        schema_version: CURRENT_SCHEMA_VERSION,
        run_id: metadata.run_id.clone(),
        vehicle_identity: metadata.vehicle_identity.clone(),
        recorded_at: UnixTimestampV1::from_live(record.recorded_at),
        payload,
    })
}

/// Project a live transition-ledger record into its archival DTO envelope.
///
/// Fails with [`ObservationError::VehicleMismatch`] if the record's vehicle identity does not
/// match the run's declared vehicle.
pub fn ledger_envelope(
    metadata: &RunMetadata,
    record: &PublishedTransitionRecord,
) -> Result<StreamEnvelopeV1<LedgerPayloadV1>, ObservationError> {
    if record.car_identity != metadata.vehicle_identity {
        return Err(ObservationError::VehicleMismatch {
            expected: metadata.vehicle_identity.clone(),
            found: record.car_identity.clone(),
        });
    }
    let session_started_at = UnixTimestampV1::from_live(record.session_started_at);
    if session_started_at != metadata.session_started_at {
        return Err(ObservationError::SessionMismatch {
            expected: metadata.session_started_at.to_string(),
            found: session_started_at.to_string(),
        });
    }

    let payload = LedgerPayloadV1 {
        session_started_at,
        record_seq: record.record_seq,
        event: project_fsm_event(&record.event),
        old_state: project_fsm_state(&record.old_state),
        next_state: project_fsm_state(&record.next_state),
        old_ctx: project_vehicle_context(&record.old_ctx),
        current_ctx: project_vehicle_context(&record.current_ctx),
        actions: record.actions.iter().map(project_domain_action).collect(),
    };
    Ok(StreamEnvelopeV1 {
        schema_version: CURRENT_SCHEMA_VERSION,
        run_id: metadata.run_id.clone(),
        vehicle_identity: metadata.vehicle_identity.clone(),
        recorded_at: UnixTimestampV1::from_live(record.recorded_at),
        payload,
    })
}

fn project_diagnostic_kind(kind: &DiagnosticKind) -> DiagnosticKindV1 {
    match kind {
        DiagnosticKind::Text { text } => DiagnosticKindV1::Text { text: text.clone() },
        DiagnosticKind::Boot => DiagnosticKindV1::Boot,
        DiagnosticKind::TimerTick => DiagnosticKindV1::TimerTick,
        DiagnosticKind::HeadlampActuationUnconfirmed { on, cause } => {
            DiagnosticKindV1::HeadlampActuationUnconfirmed {
                on: *on,
                cause: project_live_incomplete_cause(*cause),
            }
        }
        DiagnosticKind::RainChanged { raining } => {
            DiagnosticKindV1::RainChanged { raining: *raining }
        }
        DiagnosticKind::WiperMotionChanged { wiping } => {
            DiagnosticKindV1::WiperMotionChanged { wiping: *wiping }
        }
        // Task 4 gives FLCM facts their own wire variant. Preserve v1 compatibility meanwhile.
        DiagnosticKind::FlcmLampFault {
            silent,
            left_fail,
            right_fail,
        } => DiagnosticKindV1::Text {
            text: format!(
                "flcm lamp fault silent={silent} left_fail={left_fail} right_fail={right_fail}"
            ),
        },
        DiagnosticKind::ActuationFailure { action, error } => DiagnosticKindV1::ActuationFailure {
            action: action.clone(),
            error: error.clone(),
        },
        DiagnosticKind::TransitionSinkFull => DiagnosticKindV1::TransitionSinkFull,
        DiagnosticKind::TransitionSinkClosed => DiagnosticKindV1::TransitionSinkClosed,
    }
}

fn project_live_incomplete_cause(
    cause: FrontHeadlampIncompleteCause,
) -> FrontHeadlampIncompleteCauseV1 {
    match cause {
        FrontHeadlampIncompleteCause::TimedOut => FrontHeadlampIncompleteCauseV1::TimedOut,
        FrontHeadlampIncompleteCause::NegativeAck => FrontHeadlampIncompleteCauseV1::NegativeAck,
        // `FrontHeadlampIncompleteCause` is `#[non_exhaustive]` for forward-compat.
        _ => FrontHeadlampIncompleteCauseV1::TimedOut,
    }
}

fn project_diagnostic_level(level: DiagnosticLevel) -> DiagnosticLevelV1 {
    match level {
        DiagnosticLevel::Info => DiagnosticLevelV1::Info,
        DiagnosticLevel::Action => DiagnosticLevelV1::Action,
        DiagnosticLevel::Alert => DiagnosticLevelV1::Alert,
        DiagnosticLevel::Warning => DiagnosticLevelV1::Warning,
        DiagnosticLevel::Error => DiagnosticLevelV1::Error,
    }
}

fn project_fsm_event(event: &PublishedFsmEvent) -> FsmEventV1 {
    match event {
        PublishedFsmEvent::PowerOn => FsmEventV1::PowerOn,
        PublishedFsmEvent::PowerOff => FsmEventV1::PowerOff,
        PublishedFsmEvent::UpdateRpm(rpm) => FsmEventV1::UpdateRpm { rpm: *rpm },
        PublishedFsmEvent::HazardButtonChanged(pressed) => {
            FsmEventV1::HazardButtonChanged { pressed: *pressed }
        }
        PublishedFsmEvent::HazardButtonObserved(pressed) => {
            FsmEventV1::HazardButtonObserved { pressed: *pressed }
        }
        PublishedFsmEvent::LeftTurnRequestObserved(pressed) => {
            FsmEventV1::LeftTurnRequestObserved { pressed: *pressed }
        }
        PublishedFsmEvent::RightTurnRequestObserved(pressed) => {
            FsmEventV1::RightTurnRequestObserved { pressed: *pressed }
        }
        PublishedFsmEvent::UpdateAmbientLux(lux) => FsmEventV1::UpdateAmbientLux { lux: *lux },
        PublishedFsmEvent::FrontHeadlampOnAck => FsmEventV1::FrontHeadlampOnAck,
        PublishedFsmEvent::FrontHeadlampOffAck => FsmEventV1::FrontHeadlampOffAck,
        PublishedFsmEvent::FrontHeadlampActuationIncomplete { direction, cause } => {
            FsmEventV1::FrontHeadlampActuationIncomplete {
                direction: project_switch_direction(direction),
                cause: project_incomplete_cause(cause),
            }
        }
        PublishedFsmEvent::TimerTick => FsmEventV1::TimerTick,
        PublishedFsmEvent::Internal(operational) => FsmEventV1::Internal {
            operational: project_operational(operational),
        },
        PublishedFsmEvent::RainsStarted => FsmEventV1::RainsStarted,
        PublishedFsmEvent::RainsStopped => FsmEventV1::RainsStopped,
    }
}

fn project_fsm_state(state: &PublishedFsmState) -> FsmStateV1 {
    match state {
        PublishedFsmState::Off => FsmStateV1::Off,
        PublishedFsmState::PreparingToStart => FsmStateV1::PreparingToStart,
        PublishedFsmState::Idle => FsmStateV1::Idle,
        PublishedFsmState::Driving => FsmStateV1::Driving,
        PublishedFsmState::DrivingDangerously => FsmStateV1::DrivingDangerously,
        PublishedFsmState::ExtremeOperationWarning { entered_at } => {
            FsmStateV1::ExtremeOperationWarning {
                entered_at: UnixTimestampV1::from_live(*entered_at),
            }
        }
        PublishedFsmState::PreparingToStop => FsmStateV1::PreparingToStop,
    }
}

fn project_domain_action(action: &PublishedDomainAction) -> DomainActionV1 {
    match action {
        PublishedDomainAction::StartBuzzer => DomainActionV1::StartBuzzer,
        PublishedDomainAction::StopBuzzer => DomainActionV1::StopBuzzer,
        PublishedDomainAction::PublishStateSync => DomainActionV1::PublishStateSync,
        PublishedDomainAction::LogWarning(message) => DomainActionV1::LogWarning {
            message: message.clone(),
        },
        PublishedDomainAction::RequestFrontHeadlampOn => DomainActionV1::RequestFrontHeadlampOn,
        PublishedDomainAction::RequestFrontHeadlampOff => DomainActionV1::RequestFrontHeadlampOff,
        PublishedDomainAction::RequestWiperStart => DomainActionV1::RequestWiperStart,
        PublishedDomainAction::RequestWiperStop => DomainActionV1::RequestWiperStop,
        PublishedDomainAction::SetTurnLights { left_on, right_on } => {
            DomainActionV1::SetTurnLights {
                left_on: *left_on,
                right_on: *right_on,
            }
        }
    }
}

fn project_vehicle_context(ctx: &PublishedVehicleContext) -> VehicleContextV1 {
    VehicleContextV1 {
        sccm: SccmContextV1 {
            hazard_button_on: ctx.sccm.hazard_button_on.into(),
            hazard_mode_on: ctx.sccm.hazard_mode_on.into(),
        },
        bcm: BcmContextV1 {
            state: project_bcm_state(ctx.bcm.state),
            left_turn_request_on: ctx.bcm.left_turn_request_on.into(),
            right_turn_request_on: ctx.bcm.right_turn_request_on.into(),
        },
        powertrain: project_powertrain_context(&ctx.powertrain),
        health: project_health_context(&ctx.health),
        visibility: project_visibility_context(&ctx.visibility),
        weather: WeatherContextV1 {
            raining: ctx.weather.raining,
        },
        headlamp: project_headlamp_context(&ctx.headlamp),
        wiper: WiperContextV1 {
            state: project_wiper_state(ctx.wiper.state),
        },
    }
}

fn project_bcm_state(state: PublishedBcmState) -> BcmStateV1 {
    match state {
        PublishedBcmState::Off => BcmStateV1::Off,
        PublishedBcmState::Ready => BcmStateV1::Ready,
    }
}

fn project_wiper_state(state: PublishedWiperState) -> WiperStateV1 {
    match state {
        PublishedWiperState::Off => WiperStateV1::Off,
        PublishedWiperState::Ready => WiperStateV1::Ready,
        PublishedWiperState::Running => WiperStateV1::Running,
    }
}

fn project_powertrain_context(ctx: &PublishedPowertrainContext) -> PowertrainContextV1 {
    PowertrainContextV1 {
        wheel_rpm: WheelRpmV1 {
            front_left: ctx.wheel_rpm.front_left,
            front_right: ctx.wheel_rpm.front_right,
            rear_left: ctx.wheel_rpm.rear_left,
            rear_right: ctx.wheel_rpm.rear_right,
        },
        speed_kph: ctx.speed_kph,
    }
}

fn project_health_context(ctx: &PublishedHealthContext) -> HealthContextV1 {
    HealthContextV1 {
        fuel_level_pct: ctx.fuel_level_pct,
        oil_pressure_kpa: ctx.oil_pressure_kpa,
        tyre_pressure_ok: ctx.tyre_pressure_ok,
    }
}

fn project_visibility_context(ctx: &PublishedVisibilityContext) -> VisibilityContextV1 {
    VisibilityContextV1 {
        ambient_lux: ctx.ambient_lux,
    }
}

fn project_headlamp_context(ctx: &PublishedHeadlampContext) -> HeadlampContextV1 {
    HeadlampContextV1 {
        state: project_headlamp_state(&ctx.state),
        ack_pending_since: ctx.ack_pending_since.map(UnixTimestampV1::from_live),
    }
}

fn project_headlamp_state(state: &PublishedHeadlampState) -> HeadlampStateV1 {
    match state {
        PublishedHeadlampState::Off => HeadlampStateV1::Off,
        PublishedHeadlampState::Ready => HeadlampStateV1::Ready,
        PublishedHeadlampState::OnRequested => HeadlampStateV1::OnRequested,
        PublishedHeadlampState::On => HeadlampStateV1::On,
        PublishedHeadlampState::OffRequested => HeadlampStateV1::OffRequested,
    }
}

fn project_switch_direction(
    direction: &PublishedFrontHeadlampSwitchDirection,
) -> FrontHeadlampSwitchDirectionV1 {
    match direction {
        PublishedFrontHeadlampSwitchDirection::On => FrontHeadlampSwitchDirectionV1::On,
        PublishedFrontHeadlampSwitchDirection::Off => FrontHeadlampSwitchDirectionV1::Off,
    }
}

fn project_incomplete_cause(
    cause: &PublishedFrontHeadlampIncompleteCause,
) -> FrontHeadlampIncompleteCauseV1 {
    match cause {
        PublishedFrontHeadlampIncompleteCause::TimedOut => FrontHeadlampIncompleteCauseV1::TimedOut,
        PublishedFrontHeadlampIncompleteCause::NegativeAck => {
            FrontHeadlampIncompleteCauseV1::NegativeAck
        }
    }
}

fn project_operational(operational: &PublishedOperational) -> OperationalV1 {
    match operational {
        PublishedOperational::LightingUnsafe => OperationalV1::LightingUnsafe,
    }
}

/// Reconstitute a live diagnostic from an archival/live-wire envelope.
pub fn diagnostic_from_envelope(
    env: &StreamEnvelopeV1<DiagnosticPayloadV1>,
) -> Result<DiagnosticRecord, ObservationError> {
    Ok(DiagnosticRecord {
        level: live_diagnostic_level(env.payload.level),
        source: intern_source(&env.payload.source),
        kind: live_diagnostic_kind(&env.payload.kind)?,
        session_started_at: env.payload.session_started_at.to_live(),
        recorded_at: env.recorded_at.to_live(),
    })
}

/// Reconstitute a live ledger record from an archival/live-wire envelope.
pub fn ledger_from_envelope(
    env: &StreamEnvelopeV1<LedgerPayloadV1>,
) -> Result<PublishedTransitionRecord, ObservationError> {
    Ok(PublishedTransitionRecord {
        car_identity: env.vehicle_identity.clone(),
        session_started_at: env.payload.session_started_at.to_live(),
        record_seq: env.payload.record_seq,
        recorded_at: env.recorded_at.to_live(),
        event: live_fsm_event(&env.payload.event),
        old_state: live_fsm_state(&env.payload.old_state),
        next_state: live_fsm_state(&env.payload.next_state),
        old_ctx: live_vehicle_context(&env.payload.old_ctx),
        current_ctx: live_vehicle_context(&env.payload.current_ctx),
        actions: env.payload.actions.iter().map(live_domain_action).collect(),
    })
}

fn intern_source(source: &str) -> &'static str {
    static CACHE: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
    let mut guard = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(existing) = cache.get(source) {
        return existing;
    }
    let leaked: &'static str = Box::leak(source.to_owned().into_boxed_str());
    cache.insert(source.to_owned(), leaked);
    leaked
}

fn live_diagnostic_level(level: DiagnosticLevelV1) -> DiagnosticLevel {
    match level {
        DiagnosticLevelV1::Info => DiagnosticLevel::Info,
        DiagnosticLevelV1::Action => DiagnosticLevel::Action,
        DiagnosticLevelV1::Alert => DiagnosticLevel::Alert,
        DiagnosticLevelV1::Warning => DiagnosticLevel::Warning,
        DiagnosticLevelV1::Error => DiagnosticLevel::Error,
    }
}

fn live_diagnostic_kind(kind: &DiagnosticKindV1) -> Result<DiagnosticKind, ObservationError> {
    Ok(match kind {
        DiagnosticKindV1::Text { text } => DiagnosticKind::Text { text: text.clone() },
        DiagnosticKindV1::Boot => DiagnosticKind::Boot,
        DiagnosticKindV1::TimerTick => DiagnosticKind::TimerTick,
        DiagnosticKindV1::HeadlampActuationUnconfirmed { on, cause } => {
            DiagnosticKind::HeadlampActuationUnconfirmed {
                on: *on,
                cause: live_incomplete_cause_for_diagnostic(*cause),
            }
        }
        DiagnosticKindV1::RainChanged { raining } => {
            DiagnosticKind::RainChanged { raining: *raining }
        }
        DiagnosticKindV1::WiperMotionChanged { wiping } => {
            DiagnosticKind::WiperMotionChanged { wiping: *wiping }
        }
        DiagnosticKindV1::ActuationFailure { action, error } => DiagnosticKind::ActuationFailure {
            action: action.clone(),
            error: error.clone(),
        },
        DiagnosticKindV1::TransitionSinkFull => DiagnosticKind::TransitionSinkFull,
        DiagnosticKindV1::TransitionSinkClosed => DiagnosticKind::TransitionSinkClosed,
    })
}

fn live_incomplete_cause_for_diagnostic(
    cause: FrontHeadlampIncompleteCauseV1,
) -> FrontHeadlampIncompleteCause {
    match cause {
        FrontHeadlampIncompleteCauseV1::TimedOut => FrontHeadlampIncompleteCause::TimedOut,
        FrontHeadlampIncompleteCauseV1::NegativeAck => FrontHeadlampIncompleteCause::NegativeAck,
    }
}

fn live_fsm_event(event: &FsmEventV1) -> PublishedFsmEvent {
    match event {
        FsmEventV1::PowerOn => PublishedFsmEvent::PowerOn,
        FsmEventV1::PowerOff => PublishedFsmEvent::PowerOff,
        FsmEventV1::UpdateRpm { rpm } => PublishedFsmEvent::UpdateRpm(*rpm),
        FsmEventV1::HazardButtonChanged { pressed } => {
            PublishedFsmEvent::HazardButtonChanged(*pressed)
        }
        FsmEventV1::HazardButtonObserved { pressed } => {
            PublishedFsmEvent::HazardButtonObserved(*pressed)
        }
        FsmEventV1::LeftTurnRequestObserved { pressed } => {
            PublishedFsmEvent::LeftTurnRequestObserved(*pressed)
        }
        FsmEventV1::RightTurnRequestObserved { pressed } => {
            PublishedFsmEvent::RightTurnRequestObserved(*pressed)
        }
        FsmEventV1::UpdateAmbientLux { lux } => PublishedFsmEvent::UpdateAmbientLux(*lux),
        FsmEventV1::FrontHeadlampOnAck => PublishedFsmEvent::FrontHeadlampOnAck,
        FsmEventV1::FrontHeadlampOffAck => PublishedFsmEvent::FrontHeadlampOffAck,
        FsmEventV1::FrontHeadlampActuationIncomplete { direction, cause } => {
            PublishedFsmEvent::FrontHeadlampActuationIncomplete {
                direction: live_switch_direction(*direction),
                cause: live_published_incomplete_cause(*cause),
            }
        }
        FsmEventV1::TimerTick => PublishedFsmEvent::TimerTick,
        FsmEventV1::Internal { operational } => {
            PublishedFsmEvent::Internal(live_operational(*operational))
        }
        FsmEventV1::RainsStarted => PublishedFsmEvent::RainsStarted,
        FsmEventV1::RainsStopped => PublishedFsmEvent::RainsStopped,
    }
}

fn live_fsm_state(state: &FsmStateV1) -> PublishedFsmState {
    match state {
        FsmStateV1::Off => PublishedFsmState::Off,
        FsmStateV1::PreparingToStart => PublishedFsmState::PreparingToStart,
        FsmStateV1::Idle => PublishedFsmState::Idle,
        FsmStateV1::Driving => PublishedFsmState::Driving,
        FsmStateV1::DrivingDangerously => PublishedFsmState::DrivingDangerously,
        FsmStateV1::ExtremeOperationWarning { entered_at } => {
            PublishedFsmState::ExtremeOperationWarning {
                entered_at: entered_at.to_live(),
            }
        }
        FsmStateV1::PreparingToStop => PublishedFsmState::PreparingToStop,
    }
}

fn live_domain_action(action: &DomainActionV1) -> PublishedDomainAction {
    match action {
        DomainActionV1::StartBuzzer => PublishedDomainAction::StartBuzzer,
        DomainActionV1::StopBuzzer => PublishedDomainAction::StopBuzzer,
        DomainActionV1::PublishStateSync => PublishedDomainAction::PublishStateSync,
        DomainActionV1::LogWarning { message } => {
            PublishedDomainAction::LogWarning(message.clone())
        }
        DomainActionV1::RequestFrontHeadlampOn => PublishedDomainAction::RequestFrontHeadlampOn,
        DomainActionV1::RequestFrontHeadlampOff => PublishedDomainAction::RequestFrontHeadlampOff,
        DomainActionV1::RequestWiperStart => PublishedDomainAction::RequestWiperStart,
        DomainActionV1::RequestWiperStop => PublishedDomainAction::RequestWiperStop,
        DomainActionV1::SetTurnLights { left_on, right_on } => {
            PublishedDomainAction::SetTurnLights {
                left_on: *left_on,
                right_on: *right_on,
            }
        }
    }
}

fn live_vehicle_context(ctx: &VehicleContextV1) -> PublishedVehicleContext {
    PublishedVehicleContext {
        sccm: PublishedSccmContext {
            hazard_button_on: ctx.sccm.hazard_button_on.into(),
            hazard_mode_on: ctx.sccm.hazard_mode_on.into(),
        },
        bcm: PublishedBcmContext {
            state: live_bcm_state(ctx.bcm.state),
            left_turn_request_on: ctx.bcm.left_turn_request_on.into(),
            right_turn_request_on: ctx.bcm.right_turn_request_on.into(),
        },
        powertrain: PublishedPowertrainContext {
            wheel_rpm: PublishedWheelRpm {
                front_left: ctx.powertrain.wheel_rpm.front_left,
                front_right: ctx.powertrain.wheel_rpm.front_right,
                rear_left: ctx.powertrain.wheel_rpm.rear_left,
                rear_right: ctx.powertrain.wheel_rpm.rear_right,
            },
            speed_kph: ctx.powertrain.speed_kph,
        },
        health: PublishedHealthContext {
            fuel_level_pct: ctx.health.fuel_level_pct,
            oil_pressure_kpa: ctx.health.oil_pressure_kpa,
            tyre_pressure_ok: ctx.health.tyre_pressure_ok,
        },
        visibility: PublishedVisibilityContext {
            ambient_lux: ctx.visibility.ambient_lux,
        },
        weather: PublishedWeatherContext {
            raining: ctx.weather.raining,
        },
        headlamp: PublishedHeadlampContext {
            state: live_headlamp_state(ctx.headlamp.state),
            ack_pending_since: ctx.headlamp.ack_pending_since.map(|ts| ts.to_live()),
        },
        wiper: PublishedWiperContext {
            state: live_wiper_state(ctx.wiper.state),
        },
    }
}

fn live_bcm_state(state: BcmStateV1) -> PublishedBcmState {
    match state {
        BcmStateV1::Off => PublishedBcmState::Off,
        BcmStateV1::Ready => PublishedBcmState::Ready,
    }
}

fn live_wiper_state(state: WiperStateV1) -> PublishedWiperState {
    match state {
        WiperStateV1::Off => PublishedWiperState::Off,
        WiperStateV1::Ready => PublishedWiperState::Ready,
        WiperStateV1::Running => PublishedWiperState::Running,
    }
}

fn live_headlamp_state(state: HeadlampStateV1) -> PublishedHeadlampState {
    match state {
        HeadlampStateV1::Off => PublishedHeadlampState::Off,
        HeadlampStateV1::Ready => PublishedHeadlampState::Ready,
        HeadlampStateV1::OnRequested => PublishedHeadlampState::OnRequested,
        HeadlampStateV1::On => PublishedHeadlampState::On,
        HeadlampStateV1::OffRequested => PublishedHeadlampState::OffRequested,
    }
}

fn live_switch_direction(
    direction: FrontHeadlampSwitchDirectionV1,
) -> PublishedFrontHeadlampSwitchDirection {
    match direction {
        FrontHeadlampSwitchDirectionV1::On => PublishedFrontHeadlampSwitchDirection::On,
        FrontHeadlampSwitchDirectionV1::Off => PublishedFrontHeadlampSwitchDirection::Off,
    }
}

fn live_published_incomplete_cause(
    cause: FrontHeadlampIncompleteCauseV1,
) -> PublishedFrontHeadlampIncompleteCause {
    match cause {
        FrontHeadlampIncompleteCauseV1::TimedOut => PublishedFrontHeadlampIncompleteCause::TimedOut,
        FrontHeadlampIncompleteCauseV1::NegativeAck => {
            PublishedFrontHeadlampIncompleteCause::NegativeAck
        }
    }
}

fn live_operational(operational: OperationalV1) -> PublishedOperational {
    match operational {
        OperationalV1::LightingUnsafe => PublishedOperational::LightingUnsafe,
    }
}
