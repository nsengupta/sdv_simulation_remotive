//! Pure BCM observation state: independent left/right values, no hazard-to-turn policy.

use crate::fsm::DomainAction;
use crate::vehicle_state::{
    BcmContext, BcmMessage, BcmOutcome, BcmState, ObservationDisposition, ObservedBool,
};

#[test]
fn bcm_default_turn_requests_are_unknown() {
    let ctx = BcmContext::default();
    assert_eq!(ctx.left_turn_request, ObservedBool::Unknown);
    assert_eq!(ctx.right_turn_request, ObservedBool::Unknown);
}

#[test]
fn bcm_first_false_left_observation_is_initial_not_duplicate() {
    let ready = BcmContext::default()
        .on_receiving_message(BcmMessage::BecomeOn)
        .ctx;
    let message = BcmMessage::LeftTurnRequestObserved(false);
    let disposition = ready.observation_disposition(message);
    let reply = ready.on_receiving_message(message);

    assert_eq!(disposition, ObservationDisposition::Initial);
    assert_eq!(reply.ctx.left_turn_request, ObservedBool::Off);
    assert!(!reply.ctx.left_turn_request_on);
    assert!(reply.outcomes.is_empty());
}

#[test]
fn bcm_repeated_equal_left_value_is_duplicate() {
    let first =
        BcmContext::default().on_receiving_message(BcmMessage::LeftTurnRequestObserved(false));
    let message = BcmMessage::LeftTurnRequestObserved(false);
    let disposition = first.ctx.observation_disposition(message);
    let duplicate = first.ctx.on_receiving_message(message);

    assert!(matches!(
        disposition,
        ObservationDisposition::Duplicate { .. }
    ));
    assert_eq!(duplicate.ctx.left_turn_request, ObservedBool::Off);
    assert!(duplicate.outcomes.is_empty());
}

#[test]
fn bcm_complementary_right_value_is_changed() {
    let first =
        BcmContext::default().on_receiving_message(BcmMessage::RightTurnRequestObserved(false));
    let message = BcmMessage::RightTurnRequestObserved(true);
    let disposition = first.ctx.observation_disposition(message);
    let changed = first.ctx.on_receiving_message(message);

    assert!(matches!(
        disposition,
        ObservationDisposition::Changed { .. }
    ));
    assert_eq!(changed.ctx.right_turn_request, ObservedBool::On);
    assert!(changed.ctx.right_turn_request_on);
    assert!(changed.outcomes.is_empty());
}

#[test]
fn bcm_updates_left_and_right_independently() {
    let left =
        BcmContext::default().on_receiving_message(BcmMessage::LeftTurnRequestObserved(true));
    assert_eq!(left.ctx.left_turn_request, ObservedBool::On);
    assert!(left.ctx.left_turn_request_on);
    assert_eq!(left.ctx.right_turn_request, ObservedBool::Unknown);
    assert!(!left.ctx.right_turn_request_on);

    let both = left
        .ctx
        .on_receiving_message(BcmMessage::RightTurnRequestObserved(false));
    assert_eq!(both.ctx.left_turn_request, ObservedBool::On);
    assert_eq!(both.ctx.right_turn_request, ObservedBool::Off);

    let left_again = both
        .ctx
        .on_receiving_message(BcmMessage::LeftTurnRequestObserved(false));
    assert_eq!(left_again.ctx.left_turn_request, ObservedBool::Off);
    assert_eq!(left_again.ctx.right_turn_request, ObservedBool::Off);
}

#[test]
fn bcm_observation_creates_no_turn_light_outcome_or_domain_action() {
    let reply = BcmContext::default()
        .on_receiving_message(BcmMessage::BecomeOn)
        .ctx
        .on_receiving_message(BcmMessage::LeftTurnRequestObserved(true));

    assert!(
        !reply
            .outcomes
            .iter()
            .any(|outcome| matches!(outcome, BcmOutcome::TurnLightsChanged { .. }))
    );
    let no_actions: &[DomainAction] = &[];
    assert!(
        !no_actions
            .iter()
            .any(|action| matches!(action, DomainAction::SetTurnLights { .. }))
    );
}

#[test]
fn bcm_become_off_restores_readiness_without_fabricating_observed_values() {
    let observed = BcmContext::default()
        .on_receiving_message(BcmMessage::BecomeOn)
        .ctx
        .on_receiving_message(BcmMessage::LeftTurnRequestObserved(true))
        .ctx
        .on_receiving_message(BcmMessage::RightTurnRequestObserved(false));
    assert_eq!(observed.ctx.left_turn_request, ObservedBool::On);
    assert_eq!(observed.ctx.right_turn_request, ObservedBool::Off);

    let stopped = observed.ctx.on_receiving_message(BcmMessage::BecomeOff);
    assert_eq!(stopped.ctx.state, BcmState::Off);
    assert_eq!(stopped.ctx.left_turn_request, ObservedBool::On);
    assert_eq!(stopped.ctx.right_turn_request, ObservedBool::Off);
    assert_eq!(
        stopped.ctx.observation_disposition(BcmMessage::BecomeOff),
        ObservationDisposition::Lifecycle
    );

    let never_observed = BcmContext::default().on_receiving_message(BcmMessage::BecomeOff);
    assert_eq!(never_observed.ctx.state, BcmState::Off);
    assert_eq!(never_observed.ctx.left_turn_request, ObservedBool::Unknown);
    assert_eq!(never_observed.ctx.right_turn_request, ObservedBool::Unknown);
}
