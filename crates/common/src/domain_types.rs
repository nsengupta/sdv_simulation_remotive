use crate::signals::{ControlSignal, LifecycleCommand, ObservedEcuSignal, VssSignal};
pub use crate::vehicle_physics::{
    RPM_EXTREME_OPERATION_THRESHOLD, RPM_IDLE, RPM_REDLINE_THRESHOLD,
    RPM_STRESS_DURATION_THRESHOLD_SECS, SPEED_EXTREME_OPERATION_THRESHOLD_KPH,
};

use serde::{Deserialize, Serialize};

// These are your "DBC" constants.
// They are "User-Defined" for your specific vehicle platform.
pub const ID_SPEED: u32 = 0x123;
pub const ID_RPM: u32 = 0x124;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleState {
    Off,
    /// Assemblies are starting up; the car is not yet in `Idle`.
    PreparingToStart,
    Idle,
    Driving,
    #[serde(alias = "warning")]
    ExtremeOperationWarning,
    Critical,
    /// Assemblies are shutting down; the car has not yet reached `Off`.
    PreparingToStop,
}
impl Default for VehicleState {
    fn default() -> Self {
        Self::Off
    }
}

/// Canonical transport-independent input accepted by the twin.
///
/// Gateway decoders and other external adapters construct this vocabulary after validating their
/// carrier-specific payloads. [`crate::twin_runtime::connectors::IngressToFsmProjector`] then
/// translates it into the state-machine vocabulary. Actor-internal coordination messages do not
/// belong here.
#[derive(Debug, Clone)]
pub enum TwinIngressEvent {
    /// Request a primary twin lifecycle transition.
    Lifecycle(LifecycleCommand),
    /// An interpreted VSS telemetry value received from an ingress carrier.
    Telemetry(VssSignal),
    /// A validated driver control received from an ingress carrier.
    Control(ControlSignal),
    /// A validated observation received independently from an ECU.
    ObservedEcu(ObservedEcuSignal),
    /// A system-generated heartbeat or check.
    TimerTick,
    /// Emergency stop or system reset.
    SystemReset,
    /// Actuator completed the command (ingress: CAN ACK decoded at gateway).
    ///
    /// External actuator feedback uses **Confirmed/Rejected**; projection maps to
    /// [`crate::fsm::FsmEvent::FrontHeadlampOnAck`] / `OffAck`. `on_command = true` → ON path.
    FrontHeadlampCommandConfirmed { on_command: bool },
    /// Actuator rejected the command (ingress: CAN NACK decoded at gateway).
    ///
    /// Maps to [`crate::fsm::FsmEvent::FrontHeadlampActuationIncomplete`] with
    /// [`crate::fsm::FrontHeadlampIncompleteCause::NegativeAck`]. Timeout stays on `TimerTick`.
    FrontHeadlampCommandRejected { on_command: bool },
}
