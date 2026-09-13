//! **L5 public API** for L6 application binaries (gateway and integration tests).
//!
//! Gateway and other edge processes must depend on this module only — not on
//! [`crate::fsm`], [`crate::twin_runtime`], or other internal modules directly.
//! See `docs/design-notes-pyramid-layers.md`.

// --- Controller (composition root / single doorway) ---

pub use crate::twin_runtime::controller::{
    ActuationCommand, AssemblyTopology, CorrelationId, VehicleController, VehicleControllerError,
    VehicleControllerRuntimeOptions,
};

// --- Canonical twin ingress vocabulary ---

pub use crate::domain_types::{TwinIngressEvent, VehicleState};
pub use crate::signals::{ControlSignal, LifecycleCommand, ObservedEcuSignal, VssSignal};

// --- Read model (snapshots + observable assembly state) ---

pub use crate::digital_twin::CarSnapshot;
pub use crate::vehicle_state::BcmState;
/// Headlamp zone state on [`CarSnapshot::context`] (L1).
pub use crate::vehicle_state::HeadlampState;
/// Wiper zone state on [`CarSnapshot::context`] (L1).
pub use crate::vehicle_state::WiperState;

// --- Observation / optional runtime wiring ---

pub use crate::observation_records::diagnostic::sink::spawn_stdout_diagnostic_observer;
pub use crate::observation_records::diagnostic::{
    DiagnosticKind, DiagnosticLevel, DiagnosticRecord,
};
pub use crate::observation_records::transition::{
    PublishedBcmContext, PublishedBcmState, PublishedDomainAction,
    PublishedFrontHeadlampIncompleteCause, PublishedFrontHeadlampSwitchDirection,
    PublishedFsmEvent, PublishedFsmState, PublishedHeadlampContext, PublishedHeadlampState,
    PublishedHealthContext, PublishedObservedBool, PublishedOperational,
    PublishedPowertrainContext, PublishedSccmContext, PublishedTransitionRecord,
    PublishedVehicleContext, PublishedVisibilityContext, PublishedWeatherContext,
    PublishedWheelRpm, PublishedWiperContext, PublishedWiperState, UnixTimestamp,
};

// --- Headlamp ingress/egress log tokens (gateway CAN loop display) ---

pub use crate::front_headlamp_log::{
    ACK_OFF, ACK_ON, CMD_OFF, CMD_ON, MSG_ACK_OFF, MSG_ACK_ON, MSG_NACK_OFF, MSG_NACK_ON,
    MSG_REQUEST_OFF, MSG_REQUEST_ON, MSG_TIMEOUT_OFF, MSG_TIMEOUT_ON, NACK_OFF, NACK_ON,
    TIMEOUT_OFF, TIMEOUT_ON,
};

// --- Integration-test timing (prefer diagnostics assertions when sufficient) ---

pub use crate::vehicle_physics::FRONT_HEADLAMP_ON_ACK_WAIT;
