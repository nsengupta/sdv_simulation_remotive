//! Diagnostic record types — streamed car-state observability from the digital twin (L3).
//!
//! The twin emits [`DiagnosticRecord`] values through an injected sink (see [`sink`]).
//! The runtime decides who reads the RX side and how to display them.
//!
//! Each record carries the same session timing pair as the transition ledger:
//! [`session_started_at`](DiagnosticRecord::session_started_at) (when the twin started) and
//! [`recorded_at`](DiagnosticRecord::recorded_at) (when this diagnostic was emitted), both
//! projected through [`SessionClock`].
//!
//! **Wire format:** archival codecs live in the L6 `observation` crate. Do not embed protobuf or
//! JSON schema derives on the record struct itself.

pub mod sink;

use std::fmt;
use std::time::{Duration, Instant};

use super::transition::{SessionClock, UnixTimestamp};
use crate::fsm::FrontHeadlampIncompleteCause;

/// Severity classification for diagnostic records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Info,
    Action,
    Alert,
    Warning,
    Error,
}

/// Structured diagnostic facts emitted by the Twin.
///
/// This stream is **wider than any single observer surface**. Variants exist so
/// capture, contracts, engineer tools, and future Zenoh subscribers can select
/// what they need. A driver Notice (or similar) MUST filter and format — e.g.
/// hide [`DiagnosticKind::TimerTick`], ignore headlamp success (zone/context),
/// and show headlamp lines only when actuation is unconfirmed.
///
/// Emit facts only: no icons, identity prefixes, or display sentences in payloads.
/// Presentation is always receiver-side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// Free-form only when no stable variant exists yet.
    Text {
        text: String,
    },

    Boot,

    /// Heartbeat / liveness fact. Keep in the stream; Observer Notice should drop it.
    TimerTick,

    /// Twin concludes the actuator did not confirm the requested headlamp action
    /// (timeout or negative ack). Confirmed happy-path ACK is NOT a diagnostic —
    /// observers read zone/context for success.
    /// TODO: may later be encased in zone tell-back; keep as diagnostic until then.
    HeadlampActuationUnconfirmed {
        on: bool,
        cause: FrontHeadlampIncompleteCause,
    },

    /// Rain policy input changed (fact for rain↔wiper proof).
    RainChanged {
        raining: bool,
    },

    /// Wiper motion changed (fact for rain↔wiper proof). Not an actuator ACK.
    WiperMotionChanged {
        wiping: bool,
    },

    FlcmLampFault {
        silent: bool,
        left_fail: bool,
        right_fail: bool,
    },

    ActuationFailure {
        action: String,
        error: String,
    },
    TransitionSinkFull,
    TransitionSinkClosed,
}

/// A single diagnostic event emitted by the digital twin actor or its components.
///
/// Records carry the full [`DiagnosticKind`] vocabulary. Observer UIs are not 1:1 with
/// every kind — they filter and format for their audience.
#[derive(Debug, Clone)]
pub struct DiagnosticRecord {
    pub level: DiagnosticLevel,
    pub source: &'static str,
    pub kind: DiagnosticKind,
    /// When this twin run started — same anchor as ledger
    /// [`super::transition::PublishedTransitionRecord::session_started_at`].
    pub session_started_at: UnixTimestamp,
    /// When this diagnostic was recorded, projected through [`SessionClock`].
    pub recorded_at: UnixTimestamp,
}

impl DiagnosticRecord {
    /// Build a diagnostic stamped through the twin's [`SessionClock`].
    pub fn at_session(
        clock: &SessionClock,
        level: DiagnosticLevel,
        source: &'static str,
        kind: DiagnosticKind,
    ) -> Self {
        Self {
            level,
            source,
            kind,
            session_started_at: clock.session_started_at(),
            recorded_at: clock.project(&Instant::now()),
        }
    }

    /// Elapsed time since session start, on the twin clock (derivable from the emitted pair).
    pub fn elapsed_since_session(&self) -> Duration {
        elapsed_since_session(self.recorded_at, self.session_started_at)
    }
}

/// Shared with the transition ledger: `recorded_at - session_started_at`.
pub fn elapsed_since_session(
    recorded_at: UnixTimestamp,
    session_started_at: UnixTimestamp,
) -> Duration {
    recorded_at.saturating_duration_since(session_started_at)
}

/// Shorthand constructors for common twin diagnostics.
impl DiagnosticRecord {
    pub fn info(clock: &SessionClock, source: &'static str, kind: DiagnosticKind) -> Self {
        Self::at_session(clock, DiagnosticLevel::Info, source, kind)
    }

    pub fn action(clock: &SessionClock, source: &'static str, kind: DiagnosticKind) -> Self {
        Self::at_session(clock, DiagnosticLevel::Action, source, kind)
    }

    pub fn alert(clock: &SessionClock, source: &'static str, kind: DiagnosticKind) -> Self {
        Self::at_session(clock, DiagnosticLevel::Alert, source, kind)
    }

    pub fn warning(clock: &SessionClock, source: &'static str, kind: DiagnosticKind) -> Self {
        Self::at_session(clock, DiagnosticLevel::Warning, source, kind)
    }

    pub fn error(clock: &SessionClock, source: &'static str, kind: DiagnosticKind) -> Self {
        Self::at_session(clock, DiagnosticLevel::Error, source, kind)
    }
}

impl fmt::Display for DiagnosticRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Level glyph is receiver-side console formatting for the stdout observer only.
        let level = match self.level {
            DiagnosticLevel::Info => "INFO",
            DiagnosticLevel::Action => "ACTION",
            DiagnosticLevel::Alert => "ALERT",
            DiagnosticLevel::Warning => "WARNING",
            DiagnosticLevel::Error => "ERROR",
        };
        write!(
            f,
            "[{level}][{}] {}",
            self.source,
            format_kind_ascii(&self.kind)
        )
    }
}

fn format_kind_ascii(kind: &DiagnosticKind) -> String {
    match kind {
        DiagnosticKind::Text { text } => text.clone(),
        DiagnosticKind::Boot => "boot".to_owned(),
        DiagnosticKind::TimerTick => "timer tick".to_owned(),
        DiagnosticKind::HeadlampActuationUnconfirmed { on, cause } => {
            let dir = if *on { "ON" } else { "OFF" };
            let cause = match cause {
                FrontHeadlampIncompleteCause::TimedOut => "TimedOut",
                FrontHeadlampIncompleteCause::NegativeAck => "NegativeAck",
            };
            format!("headlamp actuation unconfirmed direction={dir} cause={cause}")
        }
        DiagnosticKind::RainChanged { raining } => format!("rain raining={raining}"),
        DiagnosticKind::WiperMotionChanged { wiping } => format!("wiper wiping={wiping}"),
        DiagnosticKind::FlcmLampFault {
            silent,
            left_fail,
            right_fail,
        } => {
            format!("flcm lamp fault silent={silent} left_fail={left_fail} right_fail={right_fail}")
        }
        DiagnosticKind::ActuationFailure { action, error } => {
            format!("actuation failure action={action} error={error}")
        }
        DiagnosticKind::TransitionSinkFull => "transition sink full".to_owned(),
        DiagnosticKind::TransitionSinkClosed => "transition sink closed".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsm::FrontHeadlampIncompleteCause;
    use crate::observation_records::transition::SessionClock;

    #[test]
    fn record_stores_kind_not_prose_blob() {
        let clock = SessionClock::capture();
        let rec = DiagnosticRecord::warning(
            &clock,
            "VirtualCarActor",
            DiagnosticKind::HeadlampActuationUnconfirmed {
                on: true,
                cause: FrontHeadlampIncompleteCause::TimedOut,
            },
        );
        assert!(matches!(
            rec.kind,
            DiagnosticKind::HeadlampActuationUnconfirmed {
                on: true,
                cause: FrontHeadlampIncompleteCause::TimedOut
            }
        ));
    }

    #[test]
    fn display_has_no_ack_emoji_for_unconfirmed() {
        let clock = SessionClock::capture();
        let rec = DiagnosticRecord::warning(
            &clock,
            "VirtualCarActor",
            DiagnosticKind::HeadlampActuationUnconfirmed {
                on: true,
                cause: FrontHeadlampIncompleteCause::NegativeAck,
            },
        );
        let s = rec.to_string();
        assert!(!s.contains('✅'));
        assert!(!s.contains('✓'));
        assert!(s.contains("unconfirmed"));
        assert!(s.contains("NegativeAck"));
    }
}
