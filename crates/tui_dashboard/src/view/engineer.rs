use super::{format_low_beam_status, LineRole, PaneLine};
use common::facade::{
    PublishedBcmState, PublishedFsmEvent, PublishedFsmState, PublishedObservedBool,
    PublishedTransitionRecord,
};

pub struct EngineerPane {
    pub lines: Vec<PaneLine>,
}

pub fn engineer_pane(ledger: Option<&PublishedTransitionRecord>, width: usize) -> EngineerPane {
    if ledger.is_none() {
        return EngineerPane {
            lines: [
                "Twin installed.",
                "Waiting for PowerOn on CAN.",
                "Ledger and diagnostics appear after lifecycle starts.",
            ]
            .into_iter()
            .map(|line| PaneLine::plain_fitted(LineRole::Standby, line, width))
            .collect(),
        };
    }

    let row = ledger.expect("checked above");
    let lines = vec![
        PaneLine::plain_fitted(
            LineRole::EngineerState,
            &format!("Current state: {}", format_state(&row.next_state)),
            width,
        ),
        PaneLine::plain_fitted(
            LineRole::EngineerEvent,
            &format!("Last event: {}", format_event(&row.event)),
            width,
        ),
        PaneLine::plain_fitted(LineRole::EngineerHeading, "Sub-assemblies:", width),
        PaneLine::plain_fitted(
            LineRole::EngineerAssembly,
            &format!(
                "  SCCM: Hazard {}",
                format_observed_bool(row.current_ctx.sccm.hazard_mode_on)
            ),
            width,
        ),
        PaneLine::plain_fitted(
            LineRole::EngineerAssembly,
            &format!(
                "  BCM: {}  Left {}  Right {}",
                format_bcm_state(row.current_ctx.bcm.state),
                format_observed_bool(row.current_ctx.bcm.left_turn_request_on),
                format_observed_bool(row.current_ctx.bcm.right_turn_request_on),
            ),
            width,
        ),
        PaneLine::plain_fitted(
            LineRole::EngineerAssembly,
            &format!(
                "  Low beam L: {}",
                format_low_beam_status(
                    row.current_ctx.flcm.left_low_beam_status_ok,
                    row.current_ctx.flcm.silent
                )
            ),
            width,
        ),
        PaneLine::plain_fitted(
            LineRole::EngineerAssembly,
            &format!(
                "  Low beam R: {}",
                format_low_beam_status(
                    row.current_ctx.flcm.right_low_beam_status_ok,
                    row.current_ctx.flcm.silent
                )
            ),
            width,
        ),
        PaneLine::plain_fitted(
            LineRole::EngineerAssembly,
            &format!(
                "  FLCM: {}",
                if row.current_ctx.flcm.silent {
                    "SILENT"
                } else {
                    "alive"
                }
            ),
            width,
        ),
    ];
    EngineerPane { lines }
}

fn format_state(state: &PublishedFsmState) -> String {
    match state {
        PublishedFsmState::Idle => "Cruise".to_owned(),
        PublishedFsmState::ExtremeOperationWarning { .. } => "Extreme operation warning".to_owned(),
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
        PublishedFsmEvent::RainsStarted => "RainsStarted".to_owned(),
        PublishedFsmEvent::RainsStopped => "RainsStopped".to_owned(),
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
        PublishedBcmContext, PublishedFlcmContext, PublishedHeadlampContext,
        PublishedHeadlampState, PublishedHealthContext, PublishedPowertrainContext,
        PublishedSccmContext, PublishedVehicleContext, PublishedVisibilityContext,
        PublishedWeatherContext, PublishedWheelRpm, PublishedWiperContext, PublishedWiperState,
        UnixTimestamp,
    };
    use std::time::Duration;

    fn sample_ledger() -> PublishedTransitionRecord {
        PublishedTransitionRecord {
            car_identity: "x".into(),
            session_started_at: UnixTimestamp::from_duration_since_epoch(Duration::from_secs(1)),
            record_seq: 7,
            recorded_at: UnixTimestamp::from_duration_since_epoch(Duration::from_secs(2)),
            event: PublishedFsmEvent::UpdateAmbientLux(120),
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
                state: PublishedHeadlampState::On,
                ack_pending_since: None,
            },
            wiper: PublishedWiperContext {
                state: PublishedWiperState::Off,
            },
        }
    }

    #[test]
    fn engineer_labels_idle_as_cruise() {
        let mut row = sample_ledger();
        row.next_state = PublishedFsmState::Idle;
        let pane = engineer_pane(Some(&row), 40);
        assert!(pane.lines[0].text().contains("Current state: Cruise"));
        assert!(!pane.lines[0].text().contains("Idle"));
    }

    #[test]
    fn engineer_shows_state_and_partial_assemblies() {
        let pane = engineer_pane(Some(&sample_ledger()), 40);
        assert!(pane.lines[0].text().contains("Current state: Driving"));
        assert!(pane.lines[1]
            .text()
            .contains("Last event: UpdateAmbientLux"));
        assert!(pane
            .lines
            .iter()
            .any(|l| l.text().contains("SCCM: Hazard UNKNOWN")));
        assert!(pane.lines.iter().any(|l| l.text().contains("BCM: Off")));
        assert!(!pane.lines.iter().any(|l| l.text().contains("Headlamp:")));
        assert!(!pane.lines.iter().any(|l| l.text().contains("Wiper:")));
        assert!(!pane.lines.iter().any(|l| l.text().contains("Weather:")));
        assert!(!pane.lines.iter().any(|l| l.text().contains("ROB")));
    }

    #[test]
    fn engineer_standby_before_ledger() {
        let pane = engineer_pane(None, 48);
        assert!(pane.lines[0].text().contains("Twin installed"));
    }

    fn pane_text(pane: &EngineerPane) -> String {
        pane.lines
            .iter()
            .map(PaneLine::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn engineer_sccm_line_shows_mode() {
        let mut ledger = sample_ledger();
        ledger.current_ctx.sccm.hazard_button_on = PublishedObservedBool::Off;
        ledger.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;
        let text = pane_text(&engineer_pane(Some(&ledger), 64));
        assert!(text.contains("SCCM: Hazard ON"), "{text}");
    }

    #[test]
    fn engineer_shows_sccm_bcm_readiness_and_unknown_latest_values() {
        let text = pane_text(&engineer_pane(Some(&sample_ledger()), 64));
        assert!(text.contains("SCCM: Hazard UNKNOWN"), "{text}");
        assert!(
            text.contains("BCM: Off  Left UNKNOWN  Right UNKNOWN"),
            "{text}"
        );
        assert!(text.contains("Low beam L: UNKNOWN"), "{text}");
        assert!(text.contains("Low beam R: UNKNOWN"), "{text}");
        assert!(!text.contains("Headlamp:"), "{text}");
        assert!(!text.contains("Wiper:"), "{text}");
    }

    #[test]
    fn engineer_shows_sccm_bcm_ready_and_on_off_latest_values() {
        let mut row = sample_ledger();
        row.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;
        row.current_ctx.bcm.state = PublishedBcmState::Ready;
        row.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        row.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::Off;
        let text = pane_text(&engineer_pane(Some(&row), 64));
        assert!(text.contains("SCCM: Hazard ON"), "{text}");
        assert!(text.contains("BCM: Ready  Left ON  Right OFF"), "{text}");
    }

    #[test]
    fn engineer_attended_assemblies_are_only_sccm_hazard_and_bcm_turns() {
        let mut ledger = sample_ledger();
        ledger.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;
        ledger.current_ctx.bcm.state = PublishedBcmState::Ready;
        ledger.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        ledger.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;
        ledger.current_ctx.visibility.ambient_lux = 999;
        ledger.current_ctx.weather.raining = true;
        ledger.current_ctx.headlamp.state = PublishedHeadlampState::On;
        ledger.current_ctx.wiper.state = PublishedWiperState::Running;

        let text = pane_text(&engineer_pane(Some(&ledger), 64));
        assert!(text.contains("SCCM: Hazard ON"), "{text}");
        assert!(text.contains("BCM: Ready"), "{text}");
        assert!(text.contains("Left ON"), "{text}");
        assert!(text.contains("Right ON"), "{text}");
        assert!(text.contains("Low beam L: UNKNOWN"), "{text}");
        assert!(text.contains("Low beam R: UNKNOWN"), "{text}");
        for forbidden in ["Headlamp:", "Wiper:", "Weather:", "Visibility:", "lux"] {
            assert!(!text.contains(forbidden), "found {forbidden:?} in {text}");
        }
    }

    #[test]
    fn engineer_shows_low_beam_status_from_flcm_not_headlamp() {
        let mut ledger = sample_ledger();
        ledger.current_ctx.headlamp.state = PublishedHeadlampState::On;
        ledger.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        ledger.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;
        ledger.current_ctx.flcm.left_low_beam_status_ok = PublishedObservedBool::On;
        ledger.current_ctx.flcm.right_low_beam_status_ok = PublishedObservedBool::Off;
        let text = pane_text(&engineer_pane(Some(&ledger), 64));
        assert!(text.contains("Low beam L: OK"), "{text}");
        assert!(text.contains("Low beam R: FAIL"), "{text}");
        assert!(!text.contains("Low beam L: ON"), "{text}");
        assert!(!text.contains("Low beam R: OFF"), "{text}");
        assert!(!text.contains("Headlamp:"), "{text}");
    }

    #[test]
    fn engineer_marks_low_beam_rows_stale_while_flcm_is_silent() {
        let mut ledger = sample_ledger();
        ledger.current_ctx.flcm.left_low_beam_status_ok = PublishedObservedBool::On;
        ledger.current_ctx.flcm.right_low_beam_status_ok = PublishedObservedBool::On;
        ledger.current_ctx.flcm.silent = true;

        let text = pane_text(&engineer_pane(Some(&ledger), 64));
        assert!(text.contains("Low beam L: OK (stale)"), "{text}");
        assert!(text.contains("Low beam R: OK (stale)"), "{text}");
        assert!(text.contains("FLCM: SILENT"), "{text}");
    }

    #[test]
    fn engineer_shows_flcm_alive_and_plain_status_when_not_silent() {
        let mut ledger = sample_ledger();
        ledger.current_ctx.flcm.left_low_beam_status_ok = PublishedObservedBool::On;
        ledger.current_ctx.flcm.right_low_beam_status_ok = PublishedObservedBool::Off;

        let text = pane_text(&engineer_pane(Some(&ledger), 64));
        assert!(text.contains("Low beam L: OK"), "{text}");
        assert!(text.contains("Low beam R: FAIL"), "{text}");
        assert!(!text.contains("(stale)"), "{text}");
        assert!(text.contains("FLCM: alive"), "{text}");
        assert!(!text.contains("FLCM: SILENT"), "{text}");
    }

    #[test]
    fn engineer_formats_observed_events_explicitly() {
        let cases = [
            (
                PublishedFsmEvent::HazardButtonObserved(true),
                "Last event: HazardButtonObserved(true)",
            ),
            (
                PublishedFsmEvent::LeftTurnRequestObserved(true),
                "Last event: LeftTurnRequestObserved(true)",
            ),
            (
                PublishedFsmEvent::RightTurnRequestObserved(false),
                "Last event: RightTurnRequestObserved(false)",
            ),
            (
                PublishedFsmEvent::LeftLowBeamStatusObserved(true),
                "Last event: LeftLowBeamStatusObserved(true)",
            ),
            (
                PublishedFsmEvent::RightLowBeamStatusObserved(false),
                "Last event: RightLowBeamStatusObserved(false)",
            ),
        ];
        for (event, needle) in cases {
            let mut row = sample_ledger();
            row.event = event;
            let text = pane_text(&engineer_pane(Some(&row), 64));
            assert!(text.contains(needle), "{text}");
        }
    }
}
