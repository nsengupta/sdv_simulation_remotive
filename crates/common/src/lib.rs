//! One-crate library pyramid — layer map in `docs/design-notes-pyramid-layers.md`.
//!
//! Sibling order is *dependee before dependent* (foundation first), not runtime data-flow order.
//!
//! - **L0** `vehicle_physics` — constants and pure kinematics
//! - **L1** `vehicle_state`, `domain_types`, `signals`, `front_headlamp_log`
//! - **L2** `fsm` — pure decision core (`step`, `transition_map`); imports L0/L1 only
//! - **L3** `digital_twin`, `observation_records` — twin capsule and outward-facing observation records
//! - **L4** `twin_runtime` — actor runtime (sinks live under `observation_records::{transition,diagnostic}::sink`)
//! - **L5** `facade` — public surface for gateway / L6 binaries
//!
//! Acyclic among core layers: `fsm` does not import `digital_twin` or `twin_runtime`;
//! `digital_twin` imports `fsm` and `vehicle_state`; `twin_runtime` sits above `digital_twin`.
pub mod digital_twin;
pub mod domain_types;
pub mod facade;
pub mod front_headlamp_log;
pub mod fsm;
pub mod observation_records;
pub mod signals;
pub mod twin_runtime;
pub mod vehicle_physics;
pub mod vehicle_state;

#[cfg(test)]
mod test;

pub use digital_twin::{
    CarSnapshot, DigitalTwinCar, DigitalTwinCarError, LawViolation, NotFsmVocabulary, STATE_LAWS,
    StateLaw, TwinMessage, verify_state_laws,
};
pub use domain_types::{TwinIngressEvent, VehicleState};
pub use front_headlamp_log::{
    ACK_OFF, ACK_ON, CMD_OFF, CMD_ON, MSG_ACK_OFF, MSG_ACK_ON, MSG_NACK_OFF, MSG_NACK_ON,
    MSG_REQUEST_OFF, MSG_REQUEST_ON, MSG_TIMEOUT_OFF, MSG_TIMEOUT_ON, NACK_OFF, NACK_ON,
    TIMEOUT_OFF, TIMEOUT_ON,
};
pub use observation_records::diagnostic::sink::{
    DiagnosticSink, DiagnosticSinkError, TokioMpscDiagnosticSink, diag_actuation_failure,
    diag_boot, diag_flcm_lamp_fault, diag_headlamp_actuation_unconfirmed, diag_rain_changed,
    diag_timer_tick, diag_transition_sink_closed, diag_transition_sink_full, diag_warning,
    diag_wiper_motion_changed, spawn_stdout_diagnostic_observer,
};
pub use observation_records::transition::sink::{
    TokioMpscTransitionRecordSink, TransitionRecordSink, TransitionSinkError,
};
pub use observation_records::{
    DiagnosticKind, DiagnosticLevel, DiagnosticRecord, PublishedBcmContext, PublishedBcmState,
    PublishedDomainAction, PublishedFrontHeadlampIncompleteCause,
    PublishedFrontHeadlampSwitchDirection, PublishedFsmEvent, PublishedFsmState,
    PublishedHeadlampContext, PublishedHeadlampState, PublishedHealthContext,
    PublishedObservedBool, PublishedPowertrainContext, PublishedSccmContext,
    PublishedTransitionRecord, PublishedVehicleContext, PublishedVisibilityContext,
    PublishedWeatherContext, PublishedWheelRpm, PublishedWiperContext, PublishedWiperState,
    SessionClock, elapsed_since_session,
};
pub use signals::{ControlSignal, LifecycleCommand, ObservedEcuSignal, VssSignal};
pub use twin_runtime::connectors::{IngressToFsmProjector, ProjectionError, Projector};
pub use twin_runtime::controller::{
    ActuationCommand, ActuationError, ActuationFeedback, ActuationManager, CorrelationId,
    DefaultActuationManager, VehicleController, VehicleControllerError,
    VehicleControllerRuntimeOptions,
};
pub use vehicle_physics::{
    EXTREME_OPERATION_WARNING_MESSAGE, FRONT_HEADLAMP_OFF_ACK_WAIT, FRONT_HEADLAMP_ON_ACK_WAIT,
    LUX_OFF_THRESHOLD, LUX_ON_THRESHOLD, RPM_DRIVING_THRESHOLD, RPM_EXTREME_OPERATION_THRESHOLD,
    RPM_IDLE, RPM_REDLINE_THRESHOLD, RPM_STRESS_DURATION_THRESHOLD_SECS, SPEED_BAND_GREEN_MAX_KPH,
    SPEED_BAND_YELLOW_MAX_KPH, SPEED_EXTREME_OPERATION_THRESHOLD_KPH,
    SPEED_THRESHOLD_WARNING_MESSAGE, SpeedBand, SpeedBarCell, calculate_speed_from_rpm,
    extreme_operation_active, format_speed_bar, operational_warning_active, speed_band,
    speed_bar_cells, speed_threshold_exceeded,
};
