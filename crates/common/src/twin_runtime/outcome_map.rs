//! L4: map L1 zone outcomes to L2 [`DomainAction`] for the actuation path.

use crate::fsm::DomainAction;
use crate::twin_runtime::zone_turn::ZoneOutcome;
use crate::vehicle_state::{BcmOutcome, HeadlampOutcome, WiperOutcome};

pub fn zone_outcomes_to_domain_actions(
    outcomes: impl IntoIterator<Item = ZoneOutcome>,
) -> Vec<DomainAction> {
    outcomes
        .into_iter()
        .filter_map(|o| match o {
            ZoneOutcome::Bcm(bo) => Some(bcm_outcome_to_domain_action(bo)),
            ZoneOutcome::Headlamp(ho) => headlamp_outcome_to_domain_action(ho),
            ZoneOutcome::Wiper(wo) => wiper_outcome_to_domain_action(wo),
        })
        .collect()
}

fn bcm_outcome_to_domain_action(outcome: BcmOutcome) -> DomainAction {
    match outcome {
        BcmOutcome::TurnLightsChanged { left_on, right_on } => {
            DomainAction::SetTurnLights { left_on, right_on }
        }
    }
}

fn headlamp_outcome_to_domain_action(outcome: HeadlampOutcome) -> Option<DomainAction> {
    match outcome {
        HeadlampOutcome::RequestOn => Some(DomainAction::RequestFrontHeadlampOn),
        HeadlampOutcome::RequestOff => Some(DomainAction::RequestFrontHeadlampOff),
        HeadlampOutcome::LogWarning(msg) => Some(DomainAction::LogWarning(msg)),
    }
}

fn wiper_outcome_to_domain_action(outcome: WiperOutcome) -> Option<DomainAction> {
    match outcome {
        WiperOutcome::StartWiping => Some(DomainAction::RequestWiperStart),
        WiperOutcome::StopWiping => Some(DomainAction::RequestWiperStop),
        WiperOutcome::LogWarning(msg) => Some(DomainAction::LogWarning(msg)),
    }
}
