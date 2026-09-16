//! Ledger-shaped stdout for `--print-transitions-only` (ANSI when stdout is a TTY).

use common::facade::{
    PublishedFsmEvent, PublishedFsmState, PublishedObservedBool, PublishedTransitionRecord,
};

pub fn spawn_transition_log_task(
    rx: tokio::sync::mpsc::Receiver<PublishedTransitionRecord>,
    color: bool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = rx;
        while let Some(record) = rx.recv().await {
            println!("{}", format_transition_record(&record, color));
        }
    })
}

fn format_transition_record(record: &PublishedTransitionRecord, color: bool) -> String {
    let seq = format!("seq={:<4}", record.record_seq);
    let event = format!("{:?}", record.event);
    let transition = format_state_transition(&record.old_state, &record.next_state, color);
    let actions = if record.actions.is_empty() {
        "actions=[]".to_string()
    } else {
        format!("actions={:?}", record.actions)
    };
    let headlamp = format!("headlamp={:?}", record.current_ctx.headlamp.state);
    let hazard = format!(
        "sccm.hazard_button={}  sccm.hazard_mode={}  bcm.left_turn_request={}  bcm.right_turn_request={}",
        format_observed_bool(record.current_ctx.sccm.hazard_button_on),
        format_observed_bool(record.current_ctx.sccm.hazard_mode_on),
        format_observed_bool(record.current_ctx.bcm.left_turn_request_on),
        format_observed_bool(record.current_ctx.bcm.right_turn_request_on),
    );

    if !color {
        return format!("{seq}  {event}  {transition}  {actions}  {headlamp}  {hazard}");
    }

    format!(
        "{DIM}{seq}{RESET}  {event_color}{event}{RESET}  {transition}  {action_color}{actions}{RESET}  {DIM}{headlamp}  {hazard}{RESET}",
        DIM = ansi::DIM,
        RESET = ansi::RESET,
        event_color = event_color(&record.event),
        action_color = if record.actions.is_empty() {
            ansi::DIM
        } else {
            ansi::MAGENTA
        },
    )
}

fn format_observed_bool(value: PublishedObservedBool) -> &'static str {
    match value {
        PublishedObservedBool::Unknown => "unknown",
        PublishedObservedBool::Off => "off",
        PublishedObservedBool::On => "on",
    }
}

fn format_state_transition(
    old: &PublishedFsmState,
    new: &PublishedFsmState,
    color: bool,
) -> String {
    if !color {
        return format!("{old:?} → {new:?}");
    }

    let old = format!("{old:?}");
    let next = format_next_state(new);
    format!(
        "{DIM}{old}{RESET}{DIM} → {RESET}{next}{RESET}",
        DIM = ansi::DIM,
        RESET = ansi::RESET,
    )
}

fn format_next_state(state: &PublishedFsmState) -> String {
    let label = format!("{state:?}");
    format!(
        "{paint}{BOLD}{label}{RESET}",
        paint = next_state_color(state),
        BOLD = ansi::BOLD,
        RESET = ansi::RESET,
    )
}

fn event_color(event: &PublishedFsmEvent) -> &'static str {
    match event {
        PublishedFsmEvent::Internal(_) => ansi::MAGENTA,
        PublishedFsmEvent::TimerTick => ansi::DIM,
        PublishedFsmEvent::FrontHeadlampActuationIncomplete { .. } => ansi::YELLOW,
        _ => ansi::CYAN,
    }
}

/// Highlight the **exit** operational mode (no red — reserved for errors elsewhere).
fn next_state_color(state: &PublishedFsmState) -> &'static str {
    match state {
        PublishedFsmState::Off => ansi::DIM,
        PublishedFsmState::PreparingToStart | PublishedFsmState::PreparingToStop => ansi::CYAN,
        PublishedFsmState::Idle => ansi::YELLOW,
        PublishedFsmState::Driving => ansi::GREEN,
        PublishedFsmState::DrivingDangerously => ansi::MAGENTA,
        PublishedFsmState::ExtremeOperationWarning { .. } => ansi::BRIGHT_YELLOW,
    }
}

mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const CYAN: &str = "\x1b[36m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BRIGHT_YELLOW: &str = "\x1b[93m";
    pub const GREEN: &str = "\x1b[32m";
    pub const MAGENTA: &str = "\x1b[35m";
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::facade::{
        PublishedBcmContext, PublishedBcmState, PublishedDomainAction, PublishedFlcmContext,
        PublishedFsmEvent, PublishedFsmState, PublishedHeadlampContext, PublishedHeadlampState,
        PublishedHealthContext, PublishedObservedBool, PublishedPowertrainContext,
        PublishedSccmContext, PublishedTransitionRecord, PublishedVehicleContext,
        PublishedVisibilityContext, PublishedWeatherContext, PublishedWheelRpm,
        PublishedWiperContext, PublishedWiperState, UnixTimestamp,
    };
    use std::time::Duration;

    fn sample_record() -> PublishedTransitionRecord {
        PublishedTransitionRecord {
            car_identity: "test-car".to_string(),
            session_started_at: UnixTimestamp::from_duration_since_epoch(Duration::from_nanos(1)),
            record_seq: 3,
            recorded_at: UnixTimestamp::from_duration_since_epoch(Duration::from_secs(100)),
            event: PublishedFsmEvent::UpdateAmbientLux(20),
            old_state: PublishedFsmState::Idle,
            next_state: PublishedFsmState::Driving,
            old_ctx: empty_ctx(),
            current_ctx: empty_ctx(),
            actions: vec![PublishedDomainAction::RequestFrontHeadlampOn],
        }
    }

    fn empty_ctx() -> PublishedVehicleContext {
        PublishedVehicleContext {
            sccm: PublishedSccmContext {
                hazard_button_on: PublishedObservedBool::Unknown,
                hazard_mode_on: PublishedObservedBool::Off,
            },
            bcm: PublishedBcmContext {
                state: PublishedBcmState::Off,
                left_turn_request_on: PublishedObservedBool::Unknown,
                right_turn_request_on: PublishedObservedBool::Unknown,
            },
            flcm: PublishedFlcmContext::default(),
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
    fn plain_format_includes_seq_event_and_state_arrow() {
        let line = format_transition_record(&sample_record(), false);
        assert!(line.contains("seq=3"));
        assert!(line.contains("UpdateAmbientLux(20)"));
        assert!(line.contains("Idle → Driving"));
        assert!(line.contains("RequestFrontHeadlampOn"));
    }

    #[test]
    fn plain_format_includes_hazard_sccm_and_bcm_projection() {
        let mut record = sample_record();
        record.event = PublishedFsmEvent::HazardButtonChanged(true);
        record.current_ctx.sccm.hazard_button_on = PublishedObservedBool::On;
        record.current_ctx.bcm.state = PublishedBcmState::Ready;
        record.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        record.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;

        let line = format_transition_record(&record, false);

        assert!(line.contains("sccm.hazard_button=on"));
        assert!(line.contains("sccm.hazard_mode=off"));
        assert!(line.contains("bcm.left_turn_request=on"));
        assert!(line.contains("bcm.right_turn_request=on"));
    }

    #[test]
    fn plain_format_uses_observed_event_names_and_tri_state_labels() {
        let mut hazard = sample_record();
        hazard.event = PublishedFsmEvent::HazardButtonObserved(true);
        hazard.current_ctx.sccm.hazard_button_on = PublishedObservedBool::On;
        let hazard_line = format_transition_record(&hazard, false);
        assert!(
            hazard_line.contains("HazardButtonObserved(true)"),
            "{hazard_line}"
        );
        assert!(
            hazard_line.contains("sccm.hazard_button=on"),
            "{hazard_line}"
        );
        assert!(
            hazard_line.contains("sccm.hazard_mode=off"),
            "{hazard_line}"
        );

        let mut left = sample_record();
        left.event = PublishedFsmEvent::LeftTurnRequestObserved(true);
        left.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        let left_line = format_transition_record(&left, false);
        assert!(
            left_line.contains("LeftTurnRequestObserved(true)"),
            "{left_line}"
        );
        assert!(
            left_line.contains("bcm.left_turn_request=on"),
            "{left_line}"
        );

        let mut right = sample_record();
        right.event = PublishedFsmEvent::RightTurnRequestObserved(true);
        right.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;
        let right_line = format_transition_record(&right, false);
        assert!(
            right_line.contains("RightTurnRequestObserved(true)"),
            "{right_line}"
        );
        assert!(
            right_line.contains("bcm.right_turn_request=on"),
            "{right_line}"
        );
    }

    #[test]
    fn plain_format_shows_latched_mode_when_button_wire_is_off() {
        let mut record = sample_record();
        record.event = PublishedFsmEvent::HazardButtonObserved(false);
        record.current_ctx.sccm.hazard_button_on = PublishedObservedBool::Off;
        record.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;

        let line = format_transition_record(&record, false);

        assert!(line.contains("sccm.hazard_button=off"), "{line}");
        assert!(line.contains("sccm.hazard_mode=on"), "{line}");
    }

    #[test]
    fn colored_transition_highlights_next_state_separately_from_old() {
        let transition = format_state_transition(
            &PublishedFsmState::Driving,
            &PublishedFsmState::DrivingDangerously,
            true,
        );
        assert!(transition.contains("Driving"));
        assert!(transition.contains("DrivingDangerously"));
        assert!(transition.contains("→"));
        // old dimmed, next bold+magenta — distinct escape sequences around the target state.
        assert!(transition.contains(ansi::MAGENTA));
        assert!(transition.contains(ansi::BOLD));
    }
}
