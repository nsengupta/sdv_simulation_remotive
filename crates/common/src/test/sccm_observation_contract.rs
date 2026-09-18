//! Pure SCCM observation state: store what Remotive reported, classify, emit no policy.

use crate::fsm::DomainAction;
use crate::vehicle_state::{ObservationDisposition, ObservedBool, SccmContext, SccmMessage};

#[test]
fn sccm_default_hazard_is_unknown() {
    assert_eq!(SccmContext::default().hazard_button, ObservedBool::Unknown);
}

#[test]
fn sccm_first_false_observation_is_initial_not_duplicate() {
    let reply =
        SccmContext::default().on_receiving_message(SccmMessage::HazardButtonObserved(false));

    assert_eq!(reply.disposition, ObservationDisposition::Initial);
    assert_eq!(reply.ctx.hazard_button, ObservedBool::Off);
    assert!(!reply.ctx.hazard_button_on);
}

#[test]
fn sccm_repeated_equal_value_is_duplicate() {
    let first =
        SccmContext::default().on_receiving_message(SccmMessage::HazardButtonObserved(false));
    let duplicate = first
        .ctx
        .on_receiving_message(SccmMessage::HazardButtonObserved(false));

    assert!(matches!(
        duplicate.disposition,
        ObservationDisposition::Duplicate { .. }
    ));
    assert_eq!(duplicate.ctx.hazard_button, ObservedBool::Off);
}

#[test]
fn sccm_complementary_value_is_changed() {
    let first =
        SccmContext::default().on_receiving_message(SccmMessage::HazardButtonObserved(false));
    let changed = first
        .ctx
        .on_receiving_message(SccmMessage::HazardButtonObserved(true));

    assert!(matches!(
        changed.disposition,
        ObservationDisposition::Changed { .. }
    ));
    assert_eq!(changed.ctx.hazard_button, ObservedBool::On);
    assert!(changed.ctx.hazard_button_on);
}

#[test]
fn sccm_observation_does_not_return_a_domain_action() {
    let _reply: crate::vehicle_state::SccmZoneReply =
        SccmContext::default().on_receiving_message(SccmMessage::HazardButtonObserved(true));
    let no_actions: &[DomainAction] = &[];
    assert!(no_actions.is_empty());
}

#[test]
fn sccm_default_hazard_mode_is_off() {
    assert_eq!(SccmContext::default().hazard_mode, ObservedBool::Off);
}

#[test]
fn sccm_rising_edge_toggles_mode_on_and_pulse_off_leaves_mode_on() {
    let on = SccmContext::default().on_receiving_message(SccmMessage::HazardButtonObserved(true));
    assert_eq!(on.ctx.hazard_button, ObservedBool::On);
    assert_eq!(on.ctx.hazard_mode, ObservedBool::On);

    let wire_off = on
        .ctx
        .on_receiving_message(SccmMessage::HazardButtonObserved(false));
    assert_eq!(wire_off.ctx.hazard_button, ObservedBool::Off);
    assert_eq!(wire_off.ctx.hazard_mode, ObservedBool::On);
}

#[test]
fn sccm_second_rising_edge_toggles_mode_off() {
    let after_pulse = SccmContext::default()
        .on_receiving_message(SccmMessage::HazardButtonObserved(true))
        .ctx
        .on_receiving_message(SccmMessage::HazardButtonObserved(false));
    let second = after_pulse
        .ctx
        .on_receiving_message(SccmMessage::HazardButtonObserved(true));
    assert_eq!(second.ctx.hazard_mode, ObservedBool::Off);
    assert_eq!(second.ctx.hazard_button, ObservedBool::On);
}

#[test]
fn sccm_become_on_resets_mode_off_and_button_unknown() {
    let armed = SccmContext::default()
        .on_receiving_message(SccmMessage::HazardButtonObserved(true))
        .ctx;
    let reset = armed.on_receiving_message(SccmMessage::BecomeOn);
    assert_eq!(reset.ctx.hazard_mode, ObservedBool::Off);
    assert_eq!(reset.ctx.hazard_button, ObservedBool::Unknown);
    assert_eq!(reset.disposition, ObservationDisposition::Lifecycle);
}
