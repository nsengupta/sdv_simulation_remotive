//! Operational detectors: read exit cut → optional `FsmEvent::Internal`.
//!
//! **Interim location (L4 hook):** catalog lives under `twin_runtime` for step 7a only.
//! **Target home:** `fsm/detectors/` or L2 sibling — detectors reference [`FsmState`] and
//! synthesize [`FsmEvent::Internal`]; they belong beside [`transition_map`], not in runtime
//! orchestration. See § deferred (placement + table slots).
//!
//! **Physics:** every detector (lighting now; kinematic and others later) imports predicates and
//! thresholds from [`crate::vehicle_physics`] only — same constitution as `transition_map` and
//! L3 laws. New physical rules start in L0, then flow to enforce + detect paths.
//!
//! [`detect_internal_after_hop`] is the quiescence entry point (L4 calls L2 rules here).
//! **Revisit:** per-state detector `fn` pointer in `transition_map` (default no-op) so latched
//! modes never run unrelated catalog entries.

mod lighting_unsafe;

pub use lighting_unsafe::lighting_unsafe_detector;

use crate::twin_runtime::controller::AssemblyTopology;

/// Run registered detectors against the hop exit cut; first match wins.
///
/// ObservedEcus has no headlamp actor and rejects lux, so lighting policy must not run.
/// Legacy still leaves [`lighting_unsafe_detector`] unregistered (Phase I); do not wire it
/// here while DrivingDangerously cannot recover in the demo.
pub fn detect_internal_after_hop(
    _exit_state: &crate::fsm::FsmState,
    _exit_ctx: &crate::vehicle_state::VehicleContext,
    topology: AssemblyTopology,
) -> Option<crate::fsm::FsmEvent> {
    match topology {
        AssemblyTopology::ObservedEcus | AssemblyTopology::Legacy => None,
    }
}
