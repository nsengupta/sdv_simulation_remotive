#[allow(dead_code)]
mod support;

use common::facade::{PublishedDomainAction, PublishedFsmEvent};
use observation::schema::v1::{BcmStateV1, DomainActionV1, FsmEventV1, ledger_envelope};

#[test]
fn schema_v1_round_trips_hazard_context_and_atomic_action() {
    let metadata = support::fixed_run_metadata();
    let mut live = support::sample_ledger();
    live.event = PublishedFsmEvent::HazardButtonChanged(true);
    live.current_ctx.sccm.hazard_button_on = true;
    live.current_ctx.bcm.left_turn_request_on = true;
    live.current_ctx.bcm.right_turn_request_on = true;
    live.actions = vec![PublishedDomainAction::SetTurnLights {
        left_on: true,
        right_on: true,
    }];

    let envelope = ledger_envelope(&metadata, &live).expect("hazard ledger projection");
    assert_eq!(
        envelope.payload.event,
        FsmEventV1::HazardButtonChanged { pressed: true }
    );
    assert!(envelope.payload.current_ctx.sccm.hazard_button_on);
    assert_eq!(envelope.payload.current_ctx.bcm.state, BcmStateV1::Ready);
    assert!(envelope.payload.current_ctx.bcm.left_turn_request_on);
    assert!(envelope.payload.current_ctx.bcm.right_turn_request_on);
    assert_eq!(
        envelope.payload.actions,
        vec![DomainActionV1::SetTurnLights {
            left_on: true,
            right_on: true,
        }]
    );

    let restored = observation::ledger_from_envelope(&envelope).expect("hazard round trip");
    assert_eq!(restored, live);
}
