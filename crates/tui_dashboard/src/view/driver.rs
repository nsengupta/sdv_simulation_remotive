use super::{
    MISSING, LineRole, PaneLine, Segment, SegmentContent, SegmentStyle,
};
use common::DiagnosticRecord;
use common::facade::{
    DiagnosticKind, DiagnosticLevel, PublishedObservedBool, PublishedTransitionRecord,
};
use common::fsm::FrontHeadlampIncompleteCause;
use common::vehicle_physics::{
    SPEED_EXTREME_OPERATION_THRESHOLD_KPH, speed_band, speed_bar_cells,
};
use unicode_width::UnicodeWidthStr;

pub struct DriverPane {
    pub lines: Vec<PaneLine>,
}

/// Whether an Observer Notice should adopt this diagnostic as the latest displayed notice.
///
/// The diagnostic stream is wider than the Notice surface — e.g. [`DiagnosticKind::TimerTick`]
/// stays in the stream/capture but must not overwrite the driver Notice.
pub fn should_update_notice(kind: &DiagnosticKind) -> bool {
    !matches!(kind, DiagnosticKind::TimerTick)
}

pub fn driver_pane(
    diagnostic: Option<&DiagnosticRecord>,
    ledger: Option<&PublishedTransitionRecord>,
    width: usize,
) -> DriverPane {
    if ledger.is_none() {
        return DriverPane {
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

    let lines = vec![
        PaneLine::spacer(width),
        PaneLine::spacer(width),
        notice_pane_line(diagnostic, width),
        PaneLine::spacer(width),
        speed_pane_line(ledger, width),
        PaneLine::spacer(width),
        observed_pane_line(
            "Hazard: ",
            ledger
                .map(|row| row.current_ctx.sccm.hazard_mode_on)
                .unwrap_or(PublishedObservedBool::Unknown),
            width,
        ),
        PaneLine::spacer(width),
        observed_pane_line(
            "Left request: ",
            ledger
                .map(|row| row.current_ctx.bcm.left_turn_request_on)
                .unwrap_or(PublishedObservedBool::Unknown),
            width,
        ),
        PaneLine::spacer(width),
        observed_pane_line(
            "Right request: ",
            ledger
                .map(|row| row.current_ctx.bcm.right_turn_request_on)
                .unwrap_or(PublishedObservedBool::Unknown),
            width,
        ),
        PaneLine::spacer(width),
        low_beam_pane_line(
            "Low beam L: ",
            ledger
                .map(|row| row.current_ctx.flcm.left_low_beam_status_ok)
                .unwrap_or(PublishedObservedBool::Unknown),
            width,
        ),
        PaneLine::spacer(width),
        low_beam_pane_line(
            "Low beam R: ",
            ledger
                .map(|row| row.current_ctx.flcm.right_low_beam_status_ok)
                .unwrap_or(PublishedObservedBool::Unknown),
            width,
        ),
    ];
    DriverPane { lines }
}

fn format_observed_bool(value: PublishedObservedBool) -> &'static str {
    match value {
        PublishedObservedBool::Unknown => "UNKNOWN",
        PublishedObservedBool::Off => "OFF",
        PublishedObservedBool::On => "ON",
    }
}

/// FLCM status wire: `On` = OK, `Off` = Fail, `Unknown` = not yet observed.
fn format_low_beam_status(value: PublishedObservedBool) -> &'static str {
    match value {
        PublishedObservedBool::Unknown => "UNKNOWN",
        PublishedObservedBool::Off => "FAIL",
        PublishedObservedBool::On => "OK",
    }
}

fn notice_pane_line(diagnostic: Option<&DiagnosticRecord>, width: usize) -> PaneLine {
    let body = format_notice_body(diagnostic);
    let line = PaneLine {
        role: LineRole::Notice,
        segments: vec![
            Segment {
                style: SegmentStyle::Label,
                content: SegmentContent::Text("Notice: ".to_owned()),
            },
            Segment {
                style: SegmentStyle::Default,
                content: SegmentContent::Text(body),
            },
        ],
    };
    fit_or_pad(line, width)
}

fn format_notice_body(diagnostic: Option<&DiagnosticRecord>) -> String {
    let Some(d) = diagnostic else {
        return "(no notice yet)".to_owned();
    };
    match &d.kind {
        DiagnosticKind::TimerTick => "(no notice yet)".to_owned(),
        DiagnosticKind::Boot => {
            format!("{} — Twin booting", format_level(d.level))
        }
        DiagnosticKind::HeadlampActuationUnconfirmed { .. }
        | DiagnosticKind::RainChanged { .. }
        | DiagnosticKind::WiperMotionChanged { .. } => "(no notice yet)".to_owned(),
        DiagnosticKind::FlcmLampFault {
            silent,
            left_fail,
            right_fail,
        } => {
            if d.level == DiagnosticLevel::Warning {
                format!(
                    "{} — FLCM low beam fault silent={silent} left_fail={left_fail} right_fail={right_fail}",
                    format_level(d.level)
                )
            } else {
                "(no notice yet)".to_owned()
            }
        }
        DiagnosticKind::ActuationFailure { action, error } => format!(
            "{} — Actuation failure ({action}: {error})",
            format_level(d.level)
        ),
        DiagnosticKind::TransitionSinkFull => {
            format!("{} — Transition sink full", format_level(d.level))
        }
        DiagnosticKind::TransitionSinkClosed => {
            format!("{} — Transition sink closed", format_level(d.level))
        }
        DiagnosticKind::Text { text } => {
            let msg = driver_facing_text(text);
            if msg == "Must be IDLE before POWER-OFF" {
                msg
            } else {
                format!("{} — {msg}", format_level(d.level))
            }
        }
    }
}

/// Strip twin/engineer jargon from free-form [`DiagnosticKind::Text`]; identity is on the Session bar.
fn driver_facing_text(raw: &str) -> String {
    let mut s = raw.replace('\n', " ");
    if let Some(rest) = s.strip_prefix('[') {
        if let Some(idx) = rest.find("]: ") {
            s = rest[idx + 3..].to_owned();
        }
    }
    if s.contains("must be Idle before PowerOff") {
        return "Must be IDLE before POWER-OFF".to_owned();
    }
    if let Some(rest) = s.strip_prefix("[REJECTED]: ") {
        return rest.to_owned();
    }
    s
}

fn format_level(level: DiagnosticLevel) -> &'static str {
    match level {
        DiagnosticLevel::Info => "Info",
        DiagnosticLevel::Action => "Action",
        DiagnosticLevel::Alert => "Alert",
        DiagnosticLevel::Warning => "Warning",
        DiagnosticLevel::Error => "Error",
    }
}

fn speed_pane_line(ledger: Option<&PublishedTransitionRecord>, width: usize) -> PaneLine {
    let Some(row) = ledger else {
        let line = PaneLine {
            role: LineRole::Speed,
            segments: vec![
                Segment {
                    style: SegmentStyle::Label,
                    content: SegmentContent::Text("Speed: ".to_owned()),
                },
                Segment {
                    style: SegmentStyle::Default,
                    content: SegmentContent::Text(MISSING.to_owned()),
                },
            ],
        };
        return fit_or_pad(line, width);
    };
    let speed = row.current_ctx.powertrain.speed_kph;
    let band = speed_band(speed);
    let suffix_body = format!("{speed}/{SPEED_EXTREME_OPERATION_THRESHOLD_KPH} km/h");
    let label = "Speed: ";
    let open = "[";
    let mid = "] ";
    let overhead = label.width() + open.width() + mid.width() + suffix_body.width();
    let bar_width = width.saturating_sub(overhead);
    let cells = speed_bar_cells(speed, bar_width);

    let line = PaneLine {
        role: LineRole::Speed,
        segments: vec![
            Segment {
                style: SegmentStyle::Label,
                content: SegmentContent::Text(label.to_owned()),
            },
            Segment {
                style: SegmentStyle::Default,
                content: SegmentContent::Text(open.to_owned()),
            },
            Segment {
                style: SegmentStyle::Default,
                content: SegmentContent::SpeedBar { cells },
            },
            Segment {
                style: SegmentStyle::Default,
                content: SegmentContent::Text(mid.to_owned()),
            },
            Segment {
                style: SegmentStyle::from_speed_band(band),
                content: SegmentContent::Text(suffix_body),
            },
        ],
    };
    line.pad_to_width(width)
}

fn observed_pane_line(label: &str, value: PublishedObservedBool, width: usize) -> PaneLine {
    labeled_visibility_line(label, format_observed_bool(value), width)
}

fn low_beam_pane_line(label: &str, value: PublishedObservedBool, width: usize) -> PaneLine {
    labeled_visibility_line(label, format_low_beam_status(value), width)
}

fn labeled_visibility_line(label: &str, value: &str, width: usize) -> PaneLine {
    let line = PaneLine {
        role: LineRole::Visibility,
        segments: vec![
            Segment {
                style: SegmentStyle::Label,
                content: SegmentContent::Text(label.to_owned()),
            },
            Segment {
                style: SegmentStyle::Default,
                content: SegmentContent::Text(value.to_owned()),
            },
        ],
    };
    fit_or_pad(line, width)
}

/// Pad when short; if over width, fall back to clipped plain text (glyphs stay as characters).
fn fit_or_pad(line: PaneLine, width: usize) -> PaneLine {
    if line.display_width() <= width {
        line.pad_to_width(width)
    } else {
        PaneLine::plain_fitted(line.role, &line.text(), width)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use common::facade::{
        PublishedBcmContext, PublishedBcmState, PublishedFlcmContext, PublishedFsmEvent, PublishedFsmState,
        PublishedHeadlampContext, PublishedHeadlampState, PublishedHealthContext,
        PublishedPowertrainContext, PublishedSccmContext, PublishedVehicleContext,
        PublishedVisibilityContext,
        PublishedWeatherContext, PublishedWheelRpm, PublishedWiperContext, PublishedWiperState,
        UnixTimestamp,
    };
    use common::vehicle_physics::SpeedBand;
    use std::time::Duration;

    fn sample_diag(kind: DiagnosticKind) -> DiagnosticRecord {
        DiagnosticRecord {
            level: DiagnosticLevel::Warning,
            source: "test",
            kind,
            session_started_at: UnixTimestamp::from_duration_since_epoch(Duration::from_secs(1)),
            recorded_at: UnixTimestamp::from_duration_since_epoch(Duration::from_secs(2)),
        }
    }

    fn sample_ledger(
        speed: u16,
        lux: u16,
        headlamp: PublishedHeadlampState,
    ) -> PublishedTransitionRecord {
        PublishedTransitionRecord {
            car_identity: "x".into(),
            session_started_at: UnixTimestamp::from_duration_since_epoch(Duration::from_secs(1)),
            record_seq: 1,
            recorded_at: UnixTimestamp::from_duration_since_epoch(Duration::from_secs(2)),
            event: PublishedFsmEvent::UpdateRpm(1500),
            old_state: PublishedFsmState::Idle,
            next_state: PublishedFsmState::Driving,
            old_ctx: ctx(0, 0, PublishedHeadlampState::Off),
            current_ctx: ctx(speed, lux, headlamp),
            actions: vec![],
        }
    }

    fn ctx(speed: u16, lux: u16, headlamp: PublishedHeadlampState) -> PublishedVehicleContext {
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
                speed_kph: speed,
            },
            health: PublishedHealthContext {
                fuel_level_pct: 100,
                oil_pressure_kpa: 100,
                tyre_pressure_ok: true,
            },
            visibility: PublishedVisibilityContext {
                ambient_lux: lux,
            },
            weather: PublishedWeatherContext { raining: false },
            headlamp: PublishedHeadlampContext {
                state: headlamp,
                ack_pending_since: None,
            },
            wiper: PublishedWiperContext {
                state: PublishedWiperState::Off,
            },
        }
    }

    #[test]
    fn driver_shows_notice_and_speed_bar() {
        let diag = sample_diag(DiagnosticKind::Text {
            text: "tunnel ahead".into(),
        });
        let ledger = sample_ledger(80, 150, PublishedHeadlampState::On);
        let pane = driver_pane(Some(&diag), Some(&ledger), 48);
        let notice = pane.lines.iter().find(|l| l.role == LineRole::Notice).unwrap();
        assert!(notice.text().contains("Notice: Warning"));
        assert!(notice.text().contains("tunnel ahead"));
        assert!(
            notice
                .segments
                .iter()
                .any(|s| matches!(&s.content, SegmentContent::Text(t) if t == "Notice: ")
                    && s.style == SegmentStyle::Label)
        );
        let speed_line = pane.lines.iter().find(|l| l.role == LineRole::Speed).unwrap();
        assert!(speed_line.text().starts_with("Speed: ["));
        assert!(speed_line.text().contains("80/160 km/h"));
        assert!(speed_line.text().contains('|'));
    }

    #[test]
    fn speed_line_zones_and_numeric_band_style() {
        let ledger = sample_ledger(155, 0, PublishedHeadlampState::Off);
        let pane = driver_pane(None, Some(&ledger), 64);
        let speed = pane.lines.iter().find(|l| l.role == LineRole::Speed).unwrap();
        let bar = speed
            .segments
            .iter()
            .find_map(|s| match &s.content {
                SegmentContent::SpeedBar { cells } => Some(cells),
                _ => None,
            })
            .expect("speed bar segment");
        assert!(bar.iter().any(|c| c.band == SpeedBand::Green));
        assert!(bar.iter().any(|c| c.band == SpeedBand::Yellow));
        assert!(bar.iter().any(|c| c.band == SpeedBand::Red));
        let numeric = speed
            .segments
            .iter()
            .find(|s| matches!(&s.content, SegmentContent::Text(t) if t.contains("km/h")))
            .expect("numeric suffix");
        assert_eq!(numeric.style, SegmentStyle::ZoneRed);
        assert!(
            speed
                .segments
                .iter()
                .any(|s| matches!(&s.content, SegmentContent::Text(t) if t.starts_with("Speed:"))
                    && s.style == SegmentStyle::Label)
        );
    }

    #[test]
    fn notice_formats_unconfirmed_without_icons() {
        let diag = sample_diag(DiagnosticKind::HeadlampActuationUnconfirmed {
            on: true,
            cause: FrontHeadlampIncompleteCause::TimedOut,
        });
        let ledger = sample_ledger(0, 100, PublishedHeadlampState::Ready);
        let pane = driver_pane(Some(&diag), Some(&ledger), 80);
        let notice = pane
            .lines
            .iter()
            .find(|l| l.role == LineRole::Notice)
            .unwrap()
            .text();
        let notice = notice.trim_end();
        assert!(notice.contains("(no notice yet)"));
        assert!(!notice.to_lowercase().contains("headlamp"));
        assert!(!notice.contains('✅'));
        assert!(!notice.contains('✓'));
    }

    #[test]
    fn timer_tick_does_not_update_notice() {
        assert!(!should_update_notice(&DiagnosticKind::TimerTick));
        assert!(should_update_notice(&DiagnosticKind::Boot));
        assert!(should_update_notice(
            &DiagnosticKind::HeadlampActuationUnconfirmed {
                on: false,
                cause: FrontHeadlampIncompleteCause::NegativeAck,
            }
        ));
        assert!(should_update_notice(&DiagnosticKind::FlcmLampFault {
            silent: true,
            left_fail: true,
            right_fail: false,
        }));
    }

    #[test]
    fn driver_notice_shows_flcm_lamp_fault_warning_with_facts() {
        let diag = sample_diag(DiagnosticKind::FlcmLampFault {
            silent: true,
            left_fail: true,
            right_fail: false,
        });
        let ledger = sample_ledger(10, 100, PublishedHeadlampState::On);
        let pane = driver_pane(Some(&diag), Some(&ledger), 96);
        let notice = pane
            .lines
            .iter()
            .find(|l| l.role == LineRole::Notice)
            .unwrap()
            .text();
        assert!(notice.contains("Notice: Warning"), "{notice}");
        assert!(notice.contains("FLCM low beam fault"), "{notice}");
        assert!(notice.contains("silent=true"), "{notice}");
        assert!(notice.contains("left_fail=true"), "{notice}");
        assert!(notice.contains("right_fail=false"), "{notice}");
        assert!(!notice.contains("(no notice yet)"), "{notice}");
    }

    #[test]
    fn driver_notice_clears_warning_for_info_flcm_lamp_fault() {
        let mut diag = sample_diag(DiagnosticKind::FlcmLampFault {
            silent: false,
            left_fail: false,
            right_fail: false,
        });
        diag.level = DiagnosticLevel::Info;
        let ledger = sample_ledger(10, 100, PublishedHeadlampState::On);
        let pane = driver_pane(Some(&diag), Some(&ledger), 80);
        let notice = pane
            .lines
            .iter()
            .find(|l| l.role == LineRole::Notice)
            .unwrap()
            .text();
        assert!(notice.contains("(no notice yet)"), "{notice}");
        assert!(!notice.contains("Warning"), "{notice}");
    }

    #[test]
    fn driver_notice_strips_car_id_and_rejection_jargon() {
        let diag = sample_diag(DiagnosticKind::Text {
            text: "[My-Opel-Corsa-1.4-GSi]: [REJECTED]: vehicle must be Idle before PowerOff; current state is Driving".into(),
        });
        let ledger = sample_ledger(0, 100, PublishedHeadlampState::Off);
        let pane = driver_pane(Some(&diag), Some(&ledger), 60);
        let notice = pane.lines.iter().find(|l| l.role == LineRole::Notice).unwrap();
        assert!(notice.text().starts_with("Notice: Must be IDLE before POWER-OFF"));
        assert!(!notice.text().contains("My-Opel"));
        assert!(!notice.text().contains("REJECTED"));
    }

    #[test]
    fn driver_inserts_blank_spacers_between_segments() {
        let ledger = sample_ledger(10, 100, PublishedHeadlampState::Off);
        let pane = driver_pane(None, Some(&ledger), 64);
        let roles: Vec<_> = pane.lines.iter().map(|l| l.role).collect();
        // Two blank lines before Notice for top padding.
        assert_eq!(roles[0], LineRole::Spacer);
        assert_eq!(roles[1], LineRole::Spacer);
        assert_eq!(roles[2], LineRole::Notice);
        assert!(roles.windows(2).any(|w| w[0] == LineRole::Notice && w[1] == LineRole::Spacer));
        assert!(roles.windows(2).any(|w| w[0] == LineRole::Speed && w[1] == LineRole::Spacer));
        let labels: Vec<String> = pane
            .lines
            .iter()
            .filter(|l| l.role == LineRole::Visibility)
            .map(PaneLine::text)
            .collect();
        assert!(labels.iter().any(|t| t.contains("Hazard:")));
        assert!(labels.iter().any(|t| t.contains("Left request:")));
        assert!(labels.iter().any(|t| t.contains("Right request:")));
        assert!(labels.iter().any(|t| t.contains("Low beam L:")));
        assert!(labels.iter().any(|t| t.contains("Low beam R:")));
        assert_eq!(labels.len(), 5);
    }

    #[test]
    fn format_observed_bool_uses_uppercase_labels() {
        assert_eq!(
            format_observed_bool(PublishedObservedBool::Unknown),
            "UNKNOWN"
        );
        assert_eq!(format_observed_bool(PublishedObservedBool::Off), "OFF");
        assert_eq!(format_observed_bool(PublishedObservedBool::On), "ON");
    }

    #[test]
    fn driver_observed_lines_fit_narrow_and_normal_widths_without_lamp_confirmation() {
        let mut ledger = sample_ledger(10, 150, PublishedHeadlampState::On);
        ledger.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;
        ledger.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        ledger.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;
        for width in [24usize, 64] {
            let pane = driver_pane(None, Some(&ledger), width);
            for line in &pane.lines {
                assert_eq!(line.display_width(), width, "{:?}", line.text());
                let text = line.text();
                assert!(!text.contains("Headlamp"), "{text}");
                assert!(!text.contains("lamp"), "{text}");
                assert!(!text.contains("confirmed"), "{text}");
            }
        }
    }

    #[test]
    fn driver_lines_never_exceed_width_or_wrap() {
        let diag = sample_diag(DiagnosticKind::Text {
            text: "x".repeat(200),
        });
        let ledger = sample_ledger(40, 10, PublishedHeadlampState::OnRequested);
        let pane = driver_pane(Some(&diag), Some(&ledger), 32);
        for line in &pane.lines {
            assert_eq!(line.display_width(), 32, "{:?}", line.text());
            assert!(!line.text().contains('\n'));
        }
    }

    #[test]
    fn higher_speed_fills_more_bar_cells() {
        let width = 48;
        let low = driver_pane(
            None,
            Some(&sample_ledger(40, 0, PublishedHeadlampState::Off)),
            width,
        );
        let high = driver_pane(
            None,
            Some(&sample_ledger(120, 0, PublishedHeadlampState::Off)),
            width,
        );
        let count = |pane: &DriverPane| {
            pane.lines
                .iter()
                .find(|l| l.role == LineRole::Speed)
                .unwrap()
                .text()
                .chars()
                .filter(|c| *c == '|')
                .count()
        };
        assert!(count(&high) > count(&low));
    }

    fn pane_text(pane: &DriverPane) -> String {
        pane.lines
            .iter()
            .map(PaneLine::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn driver_hazard_follows_mode_not_button_wire() {
        let mut ledger = sample_ledger(10, 100, PublishedHeadlampState::Off);
        ledger.current_ctx.sccm.hazard_button_on = PublishedObservedBool::Off;
        ledger.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;
        let text = pane_text(&driver_pane(None, Some(&ledger), 64));
        assert!(text.contains("Hazard: ON"), "{text}");
    }

    #[test]
    fn driver_shows_unknown_observed_signals_before_first_observation() {
        let ledger = sample_ledger(10, 100, PublishedHeadlampState::Off);
        let text = pane_text(&driver_pane(None, Some(&ledger), 64));
        assert!(text.contains("Hazard: UNKNOWN"), "{text}");
        assert!(text.contains("Left request: UNKNOWN"), "{text}");
        assert!(text.contains("Right request: UNKNOWN"), "{text}");
        assert!(text.contains("Low beam L: UNKNOWN"), "{text}");
        assert!(text.contains("Low beam R: UNKNOWN"), "{text}");
    }

    #[test]
    fn driver_shows_on_off_combinations_for_hazard_left_and_right() {
        let mut hazard_on = sample_ledger(10, 100, PublishedHeadlampState::On);
        hazard_on.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;
        hazard_on.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::Off;
        hazard_on.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;
        let on_off = pane_text(&driver_pane(None, Some(&hazard_on), 64));
        assert!(on_off.contains("Hazard: ON"), "{on_off}");
        assert!(on_off.contains("Left request: OFF"), "{on_off}");
        assert!(on_off.contains("Right request: ON"), "{on_off}");

        let mut all_off = sample_ledger(10, 100, PublishedHeadlampState::Off);
        all_off.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::Off;
        all_off.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        all_off.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::Off;
        let off_on = pane_text(&driver_pane(None, Some(&all_off), 64));
        assert!(off_on.contains("Hazard: OFF"), "{off_on}");
        assert!(off_on.contains("Left request: ON"), "{off_on}");
        assert!(off_on.contains("Right request: OFF"), "{off_on}");
    }

    #[test]
    fn driver_attended_labels_are_speed_hazard_turns_and_low_beam() {
        let mut ledger = sample_ledger(42, 150, PublishedHeadlampState::On);
        ledger.current_ctx.sccm.hazard_mode_on = PublishedObservedBool::On;
        ledger.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        ledger.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;
        ledger.current_ctx.flcm.left_low_beam_status_ok = PublishedObservedBool::On;
        ledger.current_ctx.flcm.right_low_beam_status_ok = PublishedObservedBool::Off;
        ledger.current_ctx.weather.raining = true;
        ledger.current_ctx.visibility.ambient_lux = 999;
        ledger.current_ctx.wiper.state = PublishedWiperState::Running;

        let text = pane_text(&driver_pane(None, Some(&ledger), 64));
        assert!(text.contains("Speed:"), "{text}");
        assert!(text.contains("42/"), "{text}");
        assert!(text.contains("Hazard: ON"), "{text}");
        assert!(text.contains("Left request: ON"), "{text}");
        assert!(text.contains("Right request: ON"), "{text}");
        assert!(text.contains("Low beam L: OK"), "{text}");
        assert!(text.contains("Low beam R: FAIL"), "{text}");
        assert!(!text.contains("Low beam L: ON"), "{text}");
        assert!(!text.contains("Low beam R: OFF"), "{text}");

        for forbidden in [
            "Visibility:",
            "Headlamps:",
            "Headlamp:",
            "Weather:",
            "Wipers:",
            "Ambient",
            "lux",
            "Raining",
            "Rain ",
        ] {
            assert!(!text.contains(forbidden), "found {forbidden:?} in {text}");
        }
    }

    #[test]
    fn driver_low_beam_rows_come_from_flcm_not_headlamp_or_bcm() {
        let mut ledger = sample_ledger(10, 100, PublishedHeadlampState::On);
        ledger.current_ctx.headlamp.state = PublishedHeadlampState::On;
        ledger.current_ctx.bcm.left_turn_request_on = PublishedObservedBool::On;
        ledger.current_ctx.bcm.right_turn_request_on = PublishedObservedBool::On;
        ledger.current_ctx.flcm.left_low_beam_status_ok = PublishedObservedBool::Off;
        ledger.current_ctx.flcm.right_low_beam_status_ok = PublishedObservedBool::Unknown;
        let text = pane_text(&driver_pane(None, Some(&ledger), 64));
        assert!(text.contains("Low beam L: FAIL"), "{text}");
        assert!(text.contains("Low beam R: UNKNOWN"), "{text}");
        assert!(!text.contains("Low beam L: ON"), "{text}");
        assert!(!text.contains("Low beam R: ON"), "{text}");
        assert!(!text.contains("Low beam L: OFF"), "{text}");
    }

    #[test]
    fn driver_notice_does_not_surface_unattended_domain_diagnostics() {
        let ledger = sample_ledger(10, 100, PublishedHeadlampState::Off);
        for kind in [
            DiagnosticKind::RainChanged { raining: true },
            DiagnosticKind::WiperMotionChanged { wiping: true },
            DiagnosticKind::HeadlampActuationUnconfirmed {
                on: true,
                cause: FrontHeadlampIncompleteCause::TimedOut,
            },
        ] {
            let pane = driver_pane(Some(&sample_diag(kind)), Some(&ledger), 64);
            let notice = pane
                .lines
                .iter()
                .find(|l| l.role == LineRole::Notice)
                .expect("active ledger should render Notice line")
                .text();
            assert!(!notice.to_lowercase().contains("rain"), "{notice}");
            assert!(!notice.to_lowercase().contains("wiper"), "{notice}");
            assert!(!notice.to_lowercase().contains("headlamp"), "{notice}");
        }
    }

    #[test]
    fn driver_active_view_omits_legacy_visibility_headlamp_weather_wiper() {
        let mut ledger = sample_ledger(10, 150, PublishedHeadlampState::On);
        ledger.current_ctx.weather.raining = true;
        ledger.current_ctx.wiper.state = PublishedWiperState::Running;
        let text = pane_text(&driver_pane(None, Some(&ledger), 64));
        assert!(!text.contains("Visibility:"), "{text}");
        assert!(!text.contains("Headlamps:"), "{text}");
        assert!(!text.contains("Weather:"), "{text}");
        assert!(!text.contains("Wipers:"), "{text}");
        assert!(!text.contains("lux"), "{text}");
        assert!(!text.contains("Raining"), "{text}");
        assert!(!text.contains("moving"), "{text}");
    }
}
