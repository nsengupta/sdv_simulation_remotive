use std::time::Instant;

use crate::fsm::{DomainAction, FsmEvent, FsmState, RawTransitionRecord};
use crate::observation_records::transition::{
    PublishedBcmState, PublishedDomainAction, PublishedTransitionRecord, SessionClock,
};
use crate::vehicle_state::{BcmState, VehicleContext};

#[test]
fn hazard_projection_contains_sccm_bcm_and_atomic_action() {
    let mut old_ctx = VehicleContext::default();
    old_ctx.bcm.state = BcmState::Ready;
    let mut current_ctx = old_ctx.clone();
    current_ctx.sccm.hazard_button_on = true;
    current_ctx.bcm.left_turn_request_on = true;
    current_ctx.bcm.right_turn_request_on = true;
    let raw = RawTransitionRecord {
        at: Instant::now(),
        event: FsmEvent::HazardButtonChanged(true),
        old_state: FsmState::Idle,
        next_state: FsmState::Idle,
        old_ctx,
        current_ctx,
        actions: vec![DomainAction::SetTurnLights {
            left_on: true,
            right_on: true,
        }],
    };

    let published =
        PublishedTransitionRecord::project(&raw, "hazard-car", 1, &SessionClock::capture());
    assert!(published.current_ctx.sccm.hazard_button_on);
    assert_eq!(published.current_ctx.bcm.state, PublishedBcmState::Ready);
    assert!(published.current_ctx.bcm.left_turn_request_on);
    assert!(published.current_ctx.bcm.right_turn_request_on);
    assert_eq!(
        published.actions,
        vec![PublishedDomainAction::SetTurnLights {
            left_on: true,
            right_on: true,
        }]
    );
}
