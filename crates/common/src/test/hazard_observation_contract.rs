use std::time::Instant;

use crate::fsm::{DomainAction, FsmEvent, FsmState, RawTransitionRecord};
use crate::observation_records::transition::{
    PublishedBcmState, PublishedDomainAction, PublishedFsmEvent, PublishedObservedBool,
    PublishedTransitionRecord, SessionClock,
};
use crate::vehicle_state::{BcmState, VehicleContext};

fn project(raw: RawTransitionRecord) -> PublishedTransitionRecord {
    PublishedTransitionRecord::project(&raw, "hazard-car", 1, &SessionClock::capture())
}

fn idle_record(
    event: FsmEvent,
    old_ctx: VehicleContext,
    current_ctx: VehicleContext,
    actions: Vec<DomainAction>,
) -> RawTransitionRecord {
    RawTransitionRecord {
        at: Instant::now(),
        event,
        old_state: FsmState::Idle,
        next_state: FsmState::Idle,
        old_ctx,
        current_ctx,
        actions,
    }
}

#[test]
fn initial_published_sccm_and_bcm_values_are_unknown() {
    let ctx = VehicleContext::default();
    let published = project(idle_record(FsmEvent::PowerOn, ctx.clone(), ctx, vec![]));

    assert_eq!(
        published.current_ctx.sccm.hazard_button_on,
        PublishedObservedBool::Unknown
    );
    assert_eq!(
        published.current_ctx.bcm.left_turn_request_on,
        PublishedObservedBool::Unknown
    );
    assert_eq!(
        published.current_ctx.bcm.right_turn_request_on,
        PublishedObservedBool::Unknown
    );
}

#[test]
fn observed_events_project_to_named_published_variants() {
    let mut current_ctx = VehicleContext::default();
    current_ctx.sccm.hazard_button = crate::vehicle_state::ObservedBool::On;
    current_ctx.bcm.left_turn_request = crate::vehicle_state::ObservedBool::On;
    current_ctx.bcm.right_turn_request = crate::vehicle_state::ObservedBool::On;

    let hazard = project(idle_record(
        FsmEvent::HazardButtonObserved(true),
        VehicleContext::default(),
        current_ctx.clone(),
        vec![],
    ));
    assert_eq!(hazard.event, PublishedFsmEvent::HazardButtonObserved(true));
    assert_eq!(
        hazard.current_ctx.sccm.hazard_button_on,
        PublishedObservedBool::On
    );

    let left = project(idle_record(
        FsmEvent::LeftTurnRequestObserved(true),
        VehicleContext::default(),
        current_ctx.clone(),
        vec![],
    ));
    assert_eq!(left.event, PublishedFsmEvent::LeftTurnRequestObserved(true));
    assert_eq!(
        left.current_ctx.bcm.left_turn_request_on,
        PublishedObservedBool::On
    );

    let right = project(idle_record(
        FsmEvent::RightTurnRequestObserved(true),
        VehicleContext::default(),
        current_ctx,
        vec![],
    ));
    assert_eq!(
        right.event,
        PublishedFsmEvent::RightTurnRequestObserved(true)
    );
    assert_eq!(
        right.current_ctx.bcm.right_turn_request_on,
        PublishedObservedBool::On
    );
}

#[test]
fn historical_hazard_button_changed_and_set_turn_lights_remain_readable() {
    let mut old_ctx = VehicleContext::default();
    old_ctx.bcm.state = BcmState::Ready;
    let mut current_ctx = old_ctx.clone();
    current_ctx.sccm.hazard_button = crate::vehicle_state::ObservedBool::On;
    current_ctx.sccm.hazard_button_on = true;
    current_ctx.bcm.left_turn_request = crate::vehicle_state::ObservedBool::On;
    current_ctx.bcm.left_turn_request_on = true;
    current_ctx.bcm.right_turn_request = crate::vehicle_state::ObservedBool::On;
    current_ctx.bcm.right_turn_request_on = true;
    let published = project(idle_record(
        FsmEvent::HazardButtonChanged(true),
        old_ctx,
        current_ctx,
        vec![DomainAction::SetTurnLights {
            left_on: true,
            right_on: true,
        }],
    ));

    assert_eq!(
        published.event,
        PublishedFsmEvent::HazardButtonChanged(true)
    );
    assert_eq!(published.current_ctx.bcm.state, PublishedBcmState::Ready);
    assert_eq!(
        published.current_ctx.sccm.hazard_button_on,
        PublishedObservedBool::On
    );
    assert_eq!(
        published.current_ctx.bcm.left_turn_request_on,
        PublishedObservedBool::On
    );
    assert_eq!(
        published.current_ctx.bcm.right_turn_request_on,
        PublishedObservedBool::On
    );
    assert_eq!(
        published.actions,
        vec![PublishedDomainAction::SetTurnLights {
            left_on: true,
            right_on: true,
        }]
    );
}
