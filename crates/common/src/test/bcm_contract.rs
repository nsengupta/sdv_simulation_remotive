use crate::vehicle_state::{BcmContext, BcmMessage, BcmOutcome, BcmState};

#[test]
fn bcm_starts_ready_and_shutdown_clears_turn_requests() {
    let started = BcmContext::default().on_receiving_message(BcmMessage::BecomeOn);
    assert_eq!(started.ctx.state, BcmState::Ready);
    assert!(!started.ctx.left_turn_request_on);
    assert!(!started.ctx.right_turn_request_on);

    let hazards_on = started
        .ctx
        .on_receiving_message(BcmMessage::HazardButtonChanged(true));
    let stopped = hazards_on.ctx.on_receiving_message(BcmMessage::BecomeOff);
    assert_eq!(stopped.ctx.state, BcmState::Off);
    assert!(!stopped.ctx.left_turn_request_on);
    assert!(!stopped.ctx.right_turn_request_on);
}

#[test]
fn bcm_mirrors_hazard_only_while_ready_and_emits_changed_pair_once() {
    let off = BcmContext::default().on_receiving_message(BcmMessage::HazardButtonChanged(true));
    assert_eq!(off.ctx, BcmContext::default());
    assert!(off.outcomes.is_empty());

    let ready = BcmContext::default()
        .on_receiving_message(BcmMessage::BecomeOn)
        .ctx;
    let changed = ready.on_receiving_message(BcmMessage::HazardButtonChanged(true));
    assert!(changed.ctx.left_turn_request_on);
    assert!(changed.ctx.right_turn_request_on);
    assert_eq!(
        changed.outcomes,
        vec![BcmOutcome::TurnLightsChanged {
            left_on: true,
            right_on: true,
        }]
    );

    let duplicate = changed
        .ctx
        .on_receiving_message(BcmMessage::HazardButtonChanged(true));
    assert!(duplicate.outcomes.is_empty());

    let cleared = duplicate
        .ctx
        .on_receiving_message(BcmMessage::HazardButtonChanged(false));
    assert_eq!(
        cleared.outcomes,
        vec![BcmOutcome::TurnLightsChanged {
            left_on: false,
            right_on: false,
        }]
    );
}
