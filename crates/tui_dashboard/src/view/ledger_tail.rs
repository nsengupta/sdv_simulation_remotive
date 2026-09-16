use super::{LineRole, PaneLine, fit_line};
use common::facade::{
    PublishedBcmState, PublishedFsmEvent, PublishedFsmState, PublishedObservedBool,
    PublishedTransitionRecord,
};
use std::collections::VecDeque;

pub const LEDGER_TAIL_N: usize = 20;

pub struct LedgerTail {
    rows: VecDeque<PublishedTransitionRecord>,
}

impl LedgerTail {
    pub fn new() -> Self {
        Self {
            rows: VecDeque::new(),
        }
    }

    pub fn push(&mut self, row: PublishedTransitionRecord) {
        self.rows.push_back(row);
        while self.rows.len() > LEDGER_TAIL_N {
            let drop_at = oldest_droppable_index(&self.rows).unwrap_or(0);
            self.rows.remove(drop_at);
        }
    }

    pub fn lines(&self, width: usize) -> Vec<PaneLine> {
        let last = self.rows.len().saturating_sub(1);
        self.rows
            .iter()
            .enumerate()
            .map(|(idx, row)| format_ledger_line(row, width, idx == last))
            .collect()
    }

 /// Fit to a visible row budget (inner pane height), keeping the newest lines.
    pub fn visible_lines(&self, width: usize, max_rows: usize) -> Vec<PaneLine> {
        let mut lines = self.lines(width);
        if max_rows == 0 {
            return Vec::new();
        }
        if lines.len() > max_rows {
            lines = lines.split_off(lines.len() - max_rows);
        }
        lines
    }
}

impl Default for LedgerTail {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
fn is_lifecycle(event: &PublishedFsmEvent) -> bool {
    matches!(
        event,
        PublishedFsmEvent::PowerOn | PublishedFsmEvent::PowerOff
    )
}

/// Prefer dropping ordinary telemetry over the most recent PowerOn / PowerOff so lifecycle
/// does not vanish under a flood of TimerTick / sensor rows.
fn oldest_droppable_index(rows: &VecDeque<PublishedTransitionRecord>) -> Option<usize> {
    let last_power_on = rows
        .iter()
        .rposition(|r| matches!(r.event, PublishedFsmEvent::PowerOn));
    let last_power_off = rows
        .iter()
        .rposition(|r| matches!(r.event, PublishedFsmEvent::PowerOff));
    (0..rows.len()).find(|&i| Some(i) != last_power_on && Some(i) != last_power_off)
}

pub fn format_ledger_line(row: &PublishedTransitionRecord, width: usize, newest: bool) -> PaneLine {
    let body = format!(
        "[{}] | {} | {} -> {} | SCCM Hazard={} | BCM {} L={} R={}",
        row.record_seq,
        format_event(&row.event),
        format_state(&row.old_state),
        format_state(&row.next_state),
        format_observed_bool(row.current_ctx.sccm.hazard_mode_on),
        format_bcm_state(row.current_ctx.bcm.state),
        format_observed_bool(row.current_ctx.bcm.left_turn_request_on),
        format_observed_bool(row.current_ctx.bcm.right_turn_request_on),
    );
    let line = if newest {
        format!("> {body}")
    } else {
        format!("  {body}")
    };
    PaneLine::plain(LineRole::LedgerRow, fit_line(&line, width))
}

fn format_state(state: &PublishedFsmState) -> String {
    match state {
        PublishedFsmState::Off => "SwitchedOff".to_owned(),
        PublishedFsmState::ExtremeOperationWarning { .. } => {
            "ExtremeOperationWarning".to_owned()
        }
        other => format!("{other:?}"),
    }
}

fn format_event(event: &PublishedFsmEvent) -> String {
    match event {
        PublishedFsmEvent::UpdateRpm(rpm) => format!("UpdateRpm({rpm})"),
        PublishedFsmEvent::UpdateAmbientLux(lux) => format!("UpdateAmbientLux({lux})"),
        PublishedFsmEvent::HazardButtonObserved(pressed) => {
            format!("HazardButtonObserved({pressed})")
        }
        PublishedFsmEvent::LeftTurnRequestObserved(pressed) => {
            format!("LeftTurnRequestObserved({pressed})")
        }
        PublishedFsmEvent::RightTurnRequestObserved(pressed) => {
            format!("RightTurnRequestObserved({pressed})")
        }
        PublishedFsmEvent::FrontHeadlampActuationIncomplete { direction, cause } => {
            format!("HeadlampIncomplete({direction:?},{cause:?})")
        }
        PublishedFsmEvent::Internal(op) => format!("Internal({op:?})"),
        other => format!("{other:?}"),
    }
}

fn format_observed_bool(value: PublishedObservedBool) -> &'static str {
    match value {
        PublishedObservedBool::Unknown => "UNKNOWN",
        PublishedObservedBool::Off => "OFF",
        PublishedObservedBool::On => "ON",
    }
}

fn format_bcm_state(state: PublishedBcmState) -> &'static str {
    match state {
        PublishedBcmState::Off => "Off",
        PublishedBcmState::Ready => "Ready",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::facade::{
        PublishedBcmContext, PublishedHeadlampContext, PublishedHeadlampState,
        PublishedHealthContext, PublishedPowertrainContext,
        PublishedSccmContext,
        PublishedVehicleContext, PublishedVisibilityContext, PublishedWeatherContext,
        PublishedWheelRpm, PublishedWiperContext, PublishedWiperState, UnixTimestamp,
    };
    use std::time::Duration;
    use unicode_width::UnicodeWidthStr;

    fn sample_with_seq(seq: u64, event: PublishedFsmEvent) -> PublishedTransitionRecord {
        PublishedTransitionRecord {
            car_identity: "x".into(),
            session_started_at: UnixTimestamp::from_duration_since_epoch(Duration::from_secs(1)),
            record_seq: seq,
            recorded_at: UnixTimestamp::from_duration_since_epoch(Duration::from_secs(2 + seq)),
            event,
            old_state: PublishedFsmState::Idle,
            next_state: PublishedFsmState::Driving,
            old_ctx: empty_ctx(),
            current_ctx: empty_ctx(),
            actions: vec![],
        }
    }

    fn empty_ctx() -> PublishedVehicleContext {
        PublishedVehicleContext {
            sccm: PublishedSccmContext {
                hazard_button_on: PublishedObservedBool::Unknown,
                hazard_mode_on: PublishedObservedBool::Unknown,
            },
            bcm: PublishedBcmContext {
                state: PublishedBcmState::Off,
                left_turn_request_on: PublishedObservedBool::Unknown,
                right_turn_request_on: PublishedObservedBool::Unknown,
            },
            powertrain: PublishedPowertrainContext {
                wheel_rpm: PublishedWheelRpm {
                    front_left: 0,
                    front_right: 0,
                    rear_left: 0,
                    rear_right: 0,
                },
                speed_kph: 0,
            },
            health: PublishedHealthContext {
                fuel_level_pct: 100,
                oil_pressure_kpa: 100,
                tyre_pressure_ok: true,
            },
            visibility: PublishedVisibilityContext { ambient_lux: 0 },
            weather: PublishedWeatherContext { raining: false },
            headlamp: PublishedHeadlampContext {
                state: PublishedHeadlampState::Off,
                ack_pending_since: None,
            },
            wiper: PublishedWiperContext {
                state: PublishedWiperState::Off,
            },
        }
    }

    #[test]
    fn ledger_tail_keeps_last_20_of_25() {
        let mut tail = LedgerTail::new();
        for seq in 1..=25 {
            tail.push(sample_with_seq(seq, PublishedFsmEvent::UpdateRpm(seq as u16)));
        }
        let lines = tail.lines(80);
        assert_eq!(lines.len(), 20);
        assert!(lines[0].text().contains("[6]"));
        assert!(lines[19].text().starts_with("> "));
        assert!(lines[19].text().contains("[25]"));
        assert!(lines[19].text().contains(" | UpdateRpm(25) | "));
        assert!(lines[19].text().contains("Idle -> Driving"));
    }

    #[test]
    fn ledger_retains_power_off_under_timer_tick_flood() {
        let mut tail = LedgerTail::new();
        tail.push(sample_with_seq(1, PublishedFsmEvent::PowerOn));
        for seq in 2..=15 {
            tail.push(sample_with_seq(seq, PublishedFsmEvent::UpdateRpm(seq as u16)));
        }
        tail.push(sample_with_seq(16, PublishedFsmEvent::PowerOff));
        for seq in 17..=40 {
            tail.push(sample_with_seq(seq, PublishedFsmEvent::TimerTick));
        }
        let text: String = tail
            .lines(100)
            .iter()
            .map(|l| l.text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("PowerOff"), "PowerOff must stay visible: {text}");
        assert!(text.contains("PowerOn"), "PowerOn must stay visible: {text}");
        assert_eq!(tail.lines(100).len(), LEDGER_TAIL_N);
    }

    #[test]
    fn ledger_lines_respect_width() {
        let mut tail = LedgerTail::new();
        tail.push(sample_with_seq(1, PublishedFsmEvent::UpdateRpm(1)));
        for line in tail.lines(24) {
            assert_eq!(line.text().width(), 24);
            assert!(!line.text().contains('\n'));
        }
    }

    #[test]
    fn is_lifecycle_helper_covers_power_edges() {
        assert!(is_lifecycle(&PublishedFsmEvent::PowerOn));
        assert!(is_lifecycle(&PublishedFsmEvent::PowerOff));
        assert!(!is_lifecycle(&PublishedFsmEvent::TimerTick));
    }

    #[test]
    fn ledger_formats_off_state_as_switched_off() {
        let mut row = sample_with_seq(1, PublishedFsmEvent::TimerTick);
        row.old_state = PublishedFsmState::PreparingToStop;
        row.next_state = PublishedFsmState::Off;
        let line = format_ledger_line(&row, 80, true).text();
        assert!(line.contains("PreparingToStop -> SwitchedOff"), "{line}");
        assert!(!line.contains("-> Off"));
    }

    #[test]
    fn ledger_formats_observed_events_explicitly() {
        let hazard = format_ledger_line(
            &sample_with_seq(3, PublishedFsmEvent::HazardButtonObserved(true)),
            100,
            true,
        )
        .text();
        assert!(hazard.contains("HazardButtonObserved(true)"), "{hazard}");

        let left = format_ledger_line(
            &sample_with_seq(4, PublishedFsmEvent::LeftTurnRequestObserved(true)),
            100,
            false,
        )
        .text();
        assert!(left.contains("LeftTurnRequestObserved(true)"), "{left}");

        let right = format_ledger_line(
            &sample_with_seq(5, PublishedFsmEvent::RightTurnRequestObserved(false)),
            100,
            false,
        )
        .text();
        assert!(
            right.contains("RightTurnRequestObserved(false)"),
            "{right}"
        );
    }

    #[test]
    fn ledger_shows_sccm_bcm_readiness_and_latest_values() {
        let mut unknown = sample_with_seq(1, PublishedFsmEvent::TimerTick);
        let unknown_line = format_ledger_line(&unknown, 120, true).text();
        assert!(
            unknown_line.contains("SCCM Hazard=UNKNOWN"),
            "{unknown_line}"
        );
        assert!(unknown_line.contains("BCM Off"), "{unknown_line}");
        assert!(unknown_line.contains("L=UNKNOWN"), "{unknown_line}");
        assert!(unknown_line.contains("R=UNKNOWN"), "{unknown_line}");

        unknown.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;
        unknown.current_ctx.bcm.state = PublishedBcmState::Ready;
        unknown.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        unknown.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::Off;
        unknown.event = PublishedFsmEvent::HazardButtonObserved(true);
        let known = format_ledger_line(&unknown, 120, true).text();
        assert!(known.contains("HazardButtonObserved(true)"), "{known}");
        assert!(known.contains("SCCM Hazard=ON"), "{known}");
        assert!(known.contains("BCM Ready"), "{known}");
        assert!(known.contains("L=ON"), "{known}");
        assert!(known.contains("R=OFF"), "{known}");
    }
}
