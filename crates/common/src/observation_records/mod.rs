//! Outward-facing observation records emitted by the digital twin (L3).
//!
//! Two kinds of observability output, apart from actuations:
//!
//! - **Transition records** ([`transition::PublishedTransitionRecord`]) — exact FSM transitions,
//! events, timing, and post-step context; deep visibility into how each turn is handled.
//! - **Diagnostic records** ([`diagnostic::DiagnosticRecord`]) — car state and context for
//! operator diagnosis (speed, health, visibility, rain, etc.).
//!
//! Record *types* live in [`transition`] and [`diagnostic`]; emission plumbing (sink traits,
//! channel adapters, stdout observer) lives in each submodule's [`transition::sink`] and
//! [`diagnostic::sink`] (L4-facing, but colocated here for discoverability).
//!
//! Wire-format (de)serialization (e.g. JSON today, Protobuf later) will be added in dedicated
//! modules when we choose formats — see placeholder notes on each record type.

pub mod diagnostic;
pub mod transition;

pub use diagnostic::{DiagnosticKind, DiagnosticLevel, DiagnosticRecord, elapsed_since_session};
pub use transition::{
    PublishedBcmContext, PublishedBcmState, PublishedDomainAction,
    PublishedFrontHeadlampIncompleteCause, PublishedFrontHeadlampSwitchDirection,
    PublishedFsmEvent, PublishedFsmState, PublishedHeadlampContext, PublishedHeadlampState,
    PublishedHealthContext, PublishedPowertrainContext, PublishedSccmContext,
    PublishedTransitionRecord, PublishedVehicleContext, PublishedVisibilityContext,
    PublishedWeatherContext, PublishedWheelRpm, PublishedWiperContext, PublishedWiperState,
    SessionClock,
};
