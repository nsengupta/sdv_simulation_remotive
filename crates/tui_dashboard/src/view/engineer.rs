use super::{LineRole, PaneLine};
use common::facade::{
    PublishedFsmEvent, PublishedFsmState, PublishedHeadlampState, PublishedTransitionRecord,
    PublishedWiperState,
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
                "  Headlamp: {}",
                format_headlamp(row.current_ctx.headlamp.state)
            ),
            width,
        ),
        PaneLine::plain_fitted(
            LineRole::EngineerAssembly,
            &format!("  Wiper: {}", format_wiper(row.current_ctx.wiper.state)),
            width,
        ),
    ];
    EngineerPane { lines }
}

fn format_state(state: &PublishedFsmState) -> String {
    match state {
        PublishedFsmState::ExtremeOperationWarning { .. } => {
            "Extreme operation warning".to_owned()
        }
        other => format!("{other:?}"),
    }
}

fn format_event(event: &PublishedFsmEvent) -> String {
    match event {
        PublishedFsmEvent::UpdateRpm(rpm) => format!("UpdateRpm({rpm})"),
        PublishedFsmEvent::UpdateAmbientLux(lux) => format!("UpdateAmbientLux({lux})"),
        PublishedFsmEvent::FrontHeadlampActuationIncomplete { direction, cause } => {
            format!("HeadlampIncomplete({direction:?},{cause:?})")
        }
        PublishedFsmEvent::Internal(op) => format!("Internal({op:?})"),
        PublishedFsmEvent::RainsStarted => "RainsStarted".to_owned(),
        PublishedFsmEvent::RainsStopped => "RainsStopped".to_owned(),
        other => format!("{other:?}"),
    }
}

fn format_headlamp(state: PublishedHeadlampState) -> &'static str {
    match state {
        PublishedHeadlampState::Off => "Off",
        PublishedHeadlampState::Ready => "Ready",
        PublishedHeadlampState::OnRequested => "On requested",
        PublishedHeadlampState::On => "On",
        PublishedHeadlampState::OffRequested => "Off requested",
    }
}

fn format_wiper(state: PublishedWiperState) -> &'static str {
    match state {
        PublishedWiperState::Off => "Off",
        PublishedWiperState::Ready => "Ready",
        PublishedWiperState::Running => "Running",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::facade::{
        PublishedBcmContext, PublishedBcmState, PublishedHeadlampContext, PublishedHealthContext,
        PublishedPowertrainContext, PublishedSccmContext, PublishedVehicleContext,
        PublishedVisibilityContext, PublishedWeatherContext, PublishedWheelRpm,
        PublishedWiperContext, PublishedWiperState, UnixTimestamp,
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
                state: PublishedHeadlampState::On,
                ack_pending_since: None,
            },
            wiper: PublishedWiperContext {
                state: PublishedWiperState::Off,
            },
        }
    }

    #[test]
    fn engineer_shows_state_and_partial_assemblies() {
        let pane = engineer_pane(Some(&sample_ledger()), 40);
        assert!(pane.lines[0].text().contains("Current state: Driving"));
        assert!(pane.lines[1].text().contains("Last event: UpdateAmbientLux"));
        assert!(pane.lines.iter().any(|l| l.text().contains("Headlamp: On")));
        assert!(pane.lines.iter().any(|l| l.text().contains("Wiper: Off")));
        assert!(!pane.lines.iter().any(|l| l.text().contains("Weather:")));
        assert!(!pane.lines.iter().any(|l| l.text().contains("ROB")));
    }

    #[test]
    fn engineer_shows_full_wiper() {
        let mut row = sample_ledger();
        row.current_ctx.wiper.state = PublishedWiperState::Ready;
        let pane = engineer_pane(Some(&row), 48);
        assert!(pane.lines.iter().any(|l| l.text().contains("Wiper: Ready")));
    }

    #[test]
    fn engineer_standby_before_ledger() {
        let pane = engineer_pane(None, 48);
        assert!(pane.lines[0].text().contains("Twin installed"));
    }
}
