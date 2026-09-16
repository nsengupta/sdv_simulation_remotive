//! Diagnostic-record sink abstraction and domain helpers (L4-facing emission plumbing).

use super::{DiagnosticKind, DiagnosticLevel, DiagnosticRecord};
use crate::fsm::FrontHeadlampIncompleteCause;
use crate::observation_records::transition::SessionClock;
use crate::vehicle_state::{FlcmContext, ObservedBool};
use tokio::sync::mpsc;

/// Abstract sink for diagnostic records emitted by the digital twin.
///
/// The twin is unconcerned with who reads the other end; the runtime injects the
/// appropriate implementation and decides on display / persistence.
pub trait DiagnosticSink: Send + Sync {
    fn try_emit(&self, record: DiagnosticRecord) -> Result<(), DiagnosticSinkError>;
}

/// Errors that can occur when emitting a diagnostic record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSinkError {
    Full,
    Closed,
}

/// Wraps a `tokio::sync::mpsc::Sender` as a [`DiagnosticSink`].
///
/// The twin calls `try_emit` (non-blocking); the receiver is on the runtime side.
pub struct TokioMpscDiagnosticSink {
    tx: mpsc::UnboundedSender<DiagnosticRecord>,
}

impl TokioMpscDiagnosticSink {
    pub fn new(tx: mpsc::UnboundedSender<DiagnosticRecord>) -> Self {
        Self { tx }
    }
}

impl DiagnosticSink for TokioMpscDiagnosticSink {
    fn try_emit(&self, record: DiagnosticRecord) -> Result<(), DiagnosticSinkError> {
        self.tx
            .send(record)
            .map_err(|_| DiagnosticSinkError::Closed)
    }
}

const SOURCE: &str = "VirtualCarActor";

pub fn diag_boot(clock: &SessionClock) -> DiagnosticRecord {
    DiagnosticRecord::info(clock, SOURCE, DiagnosticKind::Boot)
}

pub fn diag_timer_tick(clock: &SessionClock) -> DiagnosticRecord {
    DiagnosticRecord::info(clock, SOURCE, DiagnosticKind::TimerTick)
}

/// Twin concludes the actuator did not confirm the requested headlamp action.
pub fn diag_headlamp_actuation_unconfirmed(
    clock: &SessionClock,
    on: bool,
    cause: FrontHeadlampIncompleteCause,
) -> DiagnosticRecord {
    DiagnosticRecord::warning(
        clock,
        SOURCE,
        DiagnosticKind::HeadlampActuationUnconfirmed { on, cause },
    )
}

pub fn diag_rain_changed(clock: &SessionClock, raining: bool) -> DiagnosticRecord {
    DiagnosticRecord::info(clock, SOURCE, DiagnosticKind::RainChanged { raining })
}

pub fn diag_wiper_motion_changed(clock: &SessionClock, wiping: bool) -> DiagnosticRecord {
    DiagnosticRecord::info(clock, SOURCE, DiagnosticKind::WiperMotionChanged { wiping })
}

pub fn diag_flcm_lamp_fault(clock: &SessionClock, flcm: &FlcmContext) -> DiagnosticRecord {
    let kind = DiagnosticKind::FlcmLampFault {
        silent: flcm.silent,
        left_fail: flcm.left_low_beam_status == ObservedBool::Off,
        right_fail: flcm.right_low_beam_status == ObservedBool::Off,
    };
    if flcm.has_fault() {
        DiagnosticRecord::warning(clock, SOURCE, kind)
    } else {
        DiagnosticRecord::info(clock, SOURCE, kind)
    }
}

pub fn diag_actuation_failure(clock: &SessionClock, action: &str, err: &str) -> DiagnosticRecord {
    DiagnosticRecord::error(
        clock,
        SOURCE,
        DiagnosticKind::ActuationFailure {
            action: action.to_owned(),
            error: err.to_owned(),
        },
    )
}

/// Warning surfaced from a `DomainAction::LogWarning` intent (free-form until a stable kind exists).
pub fn diag_warning(clock: &SessionClock, text: impl Into<String>) -> DiagnosticRecord {
    DiagnosticRecord::warning(clock, SOURCE, DiagnosticKind::Text { text: text.into() })
}

pub fn diag_transition_sink_full(clock: &SessionClock) -> DiagnosticRecord {
    DiagnosticRecord::warning(clock, SOURCE, DiagnosticKind::TransitionSinkFull)
}

pub fn diag_transition_sink_closed(clock: &SessionClock) -> DiagnosticRecord {
    DiagnosticRecord::warning(clock, SOURCE, DiagnosticKind::TransitionSinkClosed)
}

/// Spawns a task that reads [`DiagnosticRecord`] values from `rx` and prints each
/// to stdout (or stderr for error-level). Formatting is receiver-side via [`Display`].
pub fn spawn_stdout_diagnostic_observer(
    mut rx: mpsc::UnboundedReceiver<DiagnosticRecord>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use std::io::Write;
        while let Some(record) = rx.recv().await {
            match record.level {
                DiagnosticLevel::Error | DiagnosticLevel::Alert => {
                    let _ = writeln!(std::io::stderr(), "{record}");
                }
                _ => {
                    let _ = writeln!(std::io::stdout(), "{record}");
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsm::FrontHeadlampIncompleteCause;
    use crate::observation_records::transition::SessionClock;

    #[test]
    fn unconfirmed_helper_sets_kind() {
        let clock = SessionClock::capture();
        let rec = diag_headlamp_actuation_unconfirmed(
            &clock,
            true,
            FrontHeadlampIncompleteCause::TimedOut,
        );
        assert_eq!(rec.level, DiagnosticLevel::Warning);
        assert!(matches!(
            rec.kind,
            DiagnosticKind::HeadlampActuationUnconfirmed { on: true, .. }
        ));
    }

    #[test]
    fn timer_tick_helper_has_no_identity_prose() {
        let clock = SessionClock::capture();
        let rec = diag_timer_tick(&clock);
        assert_eq!(rec.kind, DiagnosticKind::TimerTick);
    }
}
