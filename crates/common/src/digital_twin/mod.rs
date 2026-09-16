//! **L3** twin state capsule: snapshot, invariants, and actor mailbox vocabulary.
//!
//! Depends on [`crate::fsm`] (L2) for [`FsmState`] and [`FsmEvent`], and on
//! [`crate::vehicle_state`] (L1) for [`VehicleContext`]. State laws in
//! [`car_behaviour_checker`] pair L0 constants from [`crate::vehicle_physics`] with L2 enforce paths.
//!
//! The pure decision core does not import this module (no `fsm → digital_twin` edge).
//! Runtime orchestration lives in [`crate::twin_runtime`] (L4). See
//! `docs/design-notes-pyramid-layers.md`.

mod car_behaviour_checker;

pub use car_behaviour_checker::{LawViolation, STATE_LAWS, StateLaw, verify_state_laws};

use crate::fsm::{FsmEvent, FsmState};
use crate::vehicle_state::VehicleContext;
use ractor::RpcReplyPort;

/// Returned when a [`DigitalTwinCar`] cannot be constructed because a constituent is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigitalTwinCarError {
    /// Identity was empty or whitespace-only. A twin must have a non-blank identity.
    BlankIdentity,
}

impl std::fmt::Display for DigitalTwinCarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlankIdentity => write!(f, "DigitalTwinCar identity must not be blank"),
        }
    }
}

impl std::error::Error for DigitalTwinCarError {}

/// Runtime snapshot of the vehicle digital twin: identity, FSM state, and sensor context.
///
/// Fields are **private**: a `DigitalTwinCar` can only come to exist via [`Self::new`] (which
/// guarantees a non-blank identity), and after birth its mutable state can only evolve through
/// [`Self::apply_step`] — the recorded result of the pure `fsm::step`, which is the *sole*
/// state mutator (see `docs/design-notes-runtime-observation.md`). External code
/// cannot set `current_state`/`context` to arbitrary values; this makes "twin with a blank
/// identity" and "twin mutated outside the FSM step" unrepresentable rather than runtime-checked.
#[derive(Debug, Clone)]
pub struct DigitalTwinCar {
    identity: String,
    current_state: FsmState,
    context: VehicleContext,
}

impl DigitalTwinCar {
    /// Construct a twin, validating the only structurally-invalid constituent: a blank
    /// identity (empty or whitespace-only). The identity is stored trimmed. `current_state`
    /// and `context` are caller-supplied (e.g. a freshly-born twin passes `FsmState::Off` +
    /// `VehicleContext::default`).
    pub fn new(
        identity: impl Into<String>,
        current_state: FsmState,
        context: VehicleContext,
    ) -> Result<Self, DigitalTwinCarError> {
        let identity = identity.into();
        let trimmed = identity.trim();
        if trimmed.is_empty() {
            return Err(DigitalTwinCarError::BlankIdentity);
        }
        Ok(Self {
            identity: trimmed.to_owned(),
            current_state,
            context,
        })
    }

    /// The twin's (non-blank, trimmed) identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// The twin's current logical FSM state.
    pub fn current_state(&self) -> &FsmState {
        &self.current_state
    }

    /// The twin's sensor / health context.
    pub fn context(&self) -> &VehicleContext {
        &self.context
    }

    /// Evolve the twin by recording the result of a pure `fsm::step`. This is the **only**
    /// mutation path after construction, structurally enforcing that the FSM step is the sole
    /// state mutator.
    pub fn apply_step(&mut self, next_state: FsmState, context: VehicleContext) {
        self.current_state = next_state;
        self.context = context;
    }

    /// Checks identity and context invariants on a snapshot (e.g. after `GetStatus`).
    /// The "Master Guardian"
    /// Returns Ok() if all safety laws are satisfied, or an Err describing the violation.
    ///
    /// Thin wrapper over the snapshot-only *runtime* concerns (health) plus the pure
    /// [`verify_state_laws`] catalog. The identity is no longer checked here: a non-blank
    /// identity is now guaranteed by construction ([`Self::new`]). Health stays a runtime
    /// check because it is time-varying (sensors change), not a construction invariant.
    pub fn verify_all_invariants(&self) -> Result<(), String> {
        if !self.context.is_healthy() {
            return Err("vehicle context failed health invariants".to_owned());
        }

        verify_state_laws(&self.current_state, &self.context).map_err(|violations| {
            violations
                .iter()
                .map(|v| format!("{}: {}", v.law, v.detail))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }
}

/// A read-only snapshot returned by [`TwinMessage::GetStatus`].
///
/// Carries the twin plus `as_of_seq` — the ledger sequence (Counter A, `record_seq`) of the last
/// FSM event this snapshot reflects. A `GetStatus` reply is never "wrong", only *as-of* a point in
/// the event stream (Q3 / decision #2): it reflects events with sequence ≤ `as_of_seq`. Stamping it
/// makes that staleness legible and lets a consumer reconcile a snapshot against the `transition_tx`
/// ledger. `0` means no FSM event has been applied yet (freshly-born twin).
#[derive(Debug, Clone)]
pub struct CarSnapshot {
    car: DigitalTwinCar,
    as_of_seq: u64,
}

impl CarSnapshot {
    pub fn new(car: DigitalTwinCar, as_of_seq: u64) -> Self {
        Self { car, as_of_seq }
    }

    /// Ledger sequence of the last event this snapshot reflects (`0` = none applied yet).
    pub fn as_of_seq(&self) -> u64 {
        self.as_of_seq
    }

    /// The underlying twin value.
    pub fn car(&self) -> &DigitalTwinCar {
        &self.car
    }

    /// Delegating accessor: the twin's identity.
    pub fn identity(&self) -> &str {
        self.car.identity()
    }

    /// Delegating accessor: the twin's current FSM state.
    pub fn current_state(&self) -> &FsmState {
        self.car.current_state()
    }

    /// True when the brain FSM is in [`FsmState::Idle`] (legal stop-from state).
    pub fn is_idle(&self) -> bool {
        matches!(self.current_state(), FsmState::Idle)
    }

    /// Delegating accessor: the twin's sensor / health context.
    pub fn context(&self) -> &VehicleContext {
        self.car.context()
    }

    /// Delegating check: run the twin's snapshot invariants (see [`DigitalTwinCar::verify_all_invariants`]).
    pub fn verify_all_invariants(&self) -> Result<(), String> {
        self.car.verify_all_invariants()
    }
}

/// Actor mailbox vocabulary for the digital twin: FSM traffic plus request/reply such as [`Self::GetStatus`].
///
/// [`FsmEvent`] stays `Clone` and free of [`RpcReplyPort`]; embed domain events via [`Self::Fsm`].
#[derive(Debug)]
pub enum TwinMessage {
    /// Drive the FSM (`crate::fsm::step` derives context from event payloads and computes transitions).
    Fsm(FsmEvent),
    /// Zone twinlet tell-back after applying one message.
    ZoneReady {
        zone_id: crate::fsm::AssemblyId,
        turn_id: u64,
        /// Matches the `tell_attempt` on the tell that produced this reply (retry correlation).
        tell_attempt: u32,
        reply: ZoneReply,
    },
    /// Zone-initiated hop (ACK timer, future assembly deadlines) — not correlated to a brain `turn_id`.
    ZoneSpontaneous {
        zone_id: crate::fsm::AssemblyId,
        event: ZoneSpontaneousEvent,
    },
    /// Ractor deadline: zone twinlet did not tell-back in [`crate::twin_runtime::constants::ZONE_TELL_BACK_WAIT`].
    ZoneTellBackTimeout {
        zone_id: crate::fsm::AssemblyId,
        turn_id: u64,
        tell_attempt: u32,
    },
    /// Return an as-of snapshot of the twin (stamped with `as_of_seq`); does **not** call
    /// [`crate::fsm::transition`].
    GetStatus(RpcReplyPort<CarSnapshot>),
}

/// Generic zone tell-back envelope — wraps zone-specific reply types.
#[derive(Debug, Clone, PartialEq)]
pub enum ZoneReply {
    Sccm(crate::vehicle_state::SccmZoneReply),
    Bcm(crate::vehicle_state::BcmZoneReply),
    Flcm(crate::vehicle_state::FlcmZoneReply),
    Headlamp(crate::vehicle_state::HeadlampZoneReply),
    /// wiper zone reply.
    Wiper(crate::vehicle_state::WiperZoneReply),
}

impl ZoneReply {
    pub(crate) fn matches_assembly(&self, assembly_id: crate::fsm::AssemblyId) -> bool {
        matches!(
            (self, assembly_id),
            (Self::Sccm(_), crate::fsm::AssemblyId::Sccm)
                | (Self::Bcm(_), crate::fsm::AssemblyId::Bcm)
                | (Self::Flcm(_), crate::fsm::AssemblyId::Flcm)
                | (Self::Headlamp(_), crate::fsm::AssemblyId::Headlamp)
                | (Self::Wiper(_), crate::fsm::AssemblyId::Wiper)
        )
    }

    pub fn disposition(&self) -> crate::vehicle_state::ObservationDisposition {
        match self {
            Self::Sccm(reply) => reply.disposition,
            Self::Bcm(reply) => reply.disposition,
            Self::Flcm(reply) => reply.disposition,
            Self::Headlamp(_) | Self::Wiper(_) => {
                crate::vehicle_state::ObservationDisposition::Lifecycle
            }
        }
    }

    pub fn as_sccm(&self) -> Option<&crate::vehicle_state::SccmZoneReply> {
        if let ZoneReply::Sccm(r) = self {
            Some(r)
        } else {
            None
        }
    }

    pub fn as_bcm(&self) -> Option<&crate::vehicle_state::BcmZoneReply> {
        if let ZoneReply::Bcm(r) = self {
            Some(r)
        } else {
            None
        }
    }

    pub fn as_flcm(&self) -> Option<&crate::vehicle_state::FlcmZoneReply> {
        if let ZoneReply::Flcm(r) = self {
            Some(r)
        } else {
            None
        }
    }

    /// Borrow the inner [`HeadlampZoneReply`] if this is a headlamp reply.
    pub fn as_headlamp(&self) -> Option<&crate::vehicle_state::HeadlampZoneReply> {
        if let ZoneReply::Headlamp(r) = self {
            Some(r)
        } else {
            None
        }
    }

    /// Borrow the inner [`WiperZoneReply`] if this is a wiper reply.
    pub fn as_wiper(&self) -> Option<&crate::vehicle_state::WiperZoneReply> {
        if let ZoneReply::Wiper(r) = self {
            Some(r)
        } else {
            None
        }
    }
}

/// Brain-to-zone routing envelope — symmetric counterpart of [`ZoneReply`].
///
/// `zone_message_for_event` produces this; [`crate::twin_runtime::turn_barrier::TurnBarrier`]
/// stores it for retry; `tell_zone` dispatches it to the correct actor.
///
/// `pub(crate)` — not part of the external crate API.
#[derive(Debug, Clone)]
pub(crate) enum ZoneMessage {
    Sccm(crate::vehicle_state::SccmMessage),
    Bcm(crate::vehicle_state::BcmMessage),
    Flcm(crate::vehicle_state::FlcmMessage),
    Headlamp(crate::vehicle_state::HeadlampMessage),
    Wiper(crate::vehicle_state::WiperMessage),
}

/// Zone-initiated event payload (ACK timeout, future assembly deadlines).
#[derive(Debug, Clone)]
pub enum ZoneSpontaneousEvent {
    Headlamp {
        direction: crate::fsm::FrontHeadlampSwitchDirection,
        cause: crate::fsm::FrontHeadlampIncompleteCause,
        reply: crate::vehicle_state::HeadlampZoneReply,
    },
}

impl From<FsmEvent> for TwinMessage {
    fn from(evt: FsmEvent) -> Self {
        Self::Fsm(evt)
    }
}

/// Returned when a [`TwinMessage`] is not an [`FsmEvent`] wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotFsmVocabulary;

impl TryFrom<TwinMessage> for FsmEvent {
    type Error = NotFsmVocabulary;

    fn try_from(value: TwinMessage) -> Result<Self, Self::Error> {
        match value {
            TwinMessage::Fsm(e) => Ok(e),
            TwinMessage::GetStatus(_)
            | TwinMessage::ZoneReady { .. }
            | TwinMessage::ZoneSpontaneous { .. }
            | TwinMessage::ZoneTellBackTimeout { .. } => Err(NotFsmVocabulary),
        }
    }
}

impl TwinMessage {
    /// Borrow the inner [`FsmEvent`] when this message is [`Self::Fsm`].
    pub fn as_fsm_event(&self) -> Option<&FsmEvent> {
        match self {
            Self::Fsm(e) => Some(e),
            Self::GetStatus(_)
            | Self::ZoneReady { .. }
            | Self::ZoneSpontaneous { .. }
            | Self::ZoneTellBackTimeout { .. } => None,
        }
    }

    /// Take the inner [`FsmEvent`] when this message is [`Self::Fsm`].
    pub fn into_fsm_event(self) -> Option<FsmEvent> {
        match self {
            Self::Fsm(e) => Some(e),
            Self::GetStatus(_)
            | Self::ZoneReady { .. }
            | Self::ZoneSpontaneous { .. }
            | Self::ZoneTellBackTimeout { .. } => None,
        }
    }
}
