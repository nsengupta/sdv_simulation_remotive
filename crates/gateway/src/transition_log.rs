//! Ledger-shaped stdout for `--print-transitions-only` (ANSI when stdout is a TTY).

use common::facade::{PublishedFsmEvent, PublishedFsmState, PublishedTransitionRecord};

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
        "sccm.hazard_button_on={}  bcm.left_turn_request_on={}  bcm.right_turn_request_on={}",
        record.current_ctx.sccm.hazard_button_on,
        record.current_ctx.bcm.left_turn_request_on,
        record.current_ctx.bcm.right_turn_request_on,
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
        PublishedBcmContext, PublishedBcmState, PublishedDomainAction, PublishedFsmEvent,
        PublishedFsmState, PublishedHeadlampContext, PublishedHeadlampState,
        PublishedHealthContext, PublishedPowertrainContext, PublishedSccmContext,
        PublishedTransitionRecord, PublishedVehicleContext, PublishedVisibilityContext,
        PublishedWeatherContext, PublishedWheelRpm, PublishedWiperContext, PublishedWiperState,
        UnixTimestamp,
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
                hazard_button_on: false,
            },
            bcm: PublishedBcmContext {
                state: PublishedBcmState::Off,
                left_turn_request_on: false,
                right_turn_request_on: false,
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
        record.current_ctx.sccm.hazard_button_on = true;
        record.current_ctx.bcm.state = PublishedBcmState::Ready;
        record.current_ctx.bcm.left_turn_request_on = true;
        record.current_ctx.bcm.right_turn_request_on = true;

        let line = format_transition_record(&record, false);

        assert!(line.contains("sccm.hazard_button_on=true"));
        assert!(line.contains("bcm.left_turn_request_on=true"));
        assert!(line.contains("bcm.right_turn_request_on=true"));
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
