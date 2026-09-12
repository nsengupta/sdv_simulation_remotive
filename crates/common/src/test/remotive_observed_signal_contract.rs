//! Contract tests for independent Remotive observations on strict internal CAN carriers.

use crate::signals::{ID_HAZARD, ID_LEFT_TURN_REQUEST, ID_RIGHT_TURN_REQUEST, ObservedEcuSignal};
use socketcan::{CanFrame, EmbeddedFrame, ExtendedId, Frame, StandardId};

fn standard_frame(id: u16, data: &[u8]) -> CanFrame {
    CanFrame::new(StandardId::new(id).expect("standard CAN id"), data).expect("CAN frame")
}

#[test]
fn remotive_hazard_preserves_the_phase_one_internal_carrier() {
    let frame = ObservedEcuSignal::HazardButton(true)
        .to_can_frame()
        .expect("encode hazard");

    assert_eq!(frame.raw_id(), ID_HAZARD as u32);
    assert_eq!(frame.data(), &[1, 0]);
    assert_eq!(
        ObservedEcuSignal::from_can_frame(&frame),
        Some(ObservedEcuSignal::HazardButton(true))
    );
}

#[test]
fn remotive_left_request_has_an_independent_internal_carrier() {
    let frame = ObservedEcuSignal::LeftTurnRequest(true)
        .to_can_frame()
        .expect("encode left request");

    assert_eq!(frame.raw_id(), ID_LEFT_TURN_REQUEST as u32);
    assert_eq!(frame.data(), &[1, 0]);
}

#[test]
fn remotive_right_request_has_an_independent_internal_carrier() {
    let frame = ObservedEcuSignal::RightTurnRequest(true)
        .to_can_frame()
        .expect("encode right request");

    assert_eq!(frame.raw_id(), ID_RIGHT_TURN_REQUEST as u32);
    assert_eq!(frame.data(), &[1, 0]);
}

#[test]
fn every_observed_signal_round_trips_both_boolean_values() {
    for signal in [
        ObservedEcuSignal::HazardButton(false),
        ObservedEcuSignal::HazardButton(true),
        ObservedEcuSignal::LeftTurnRequest(false),
        ObservedEcuSignal::LeftTurnRequest(true),
        ObservedEcuSignal::RightTurnRequest(false),
        ObservedEcuSignal::RightTurnRequest(true),
    ] {
        let frame = signal.to_can_frame().expect("encode observation");
        assert_eq!(ObservedEcuSignal::from_can_frame(&frame), Some(signal));
    }
}

#[test]
fn observed_signal_decoder_rejects_invalid_value_and_reserved_byte() {
    for id in [ID_HAZARD, ID_LEFT_TURN_REQUEST, ID_RIGHT_TURN_REQUEST] {
        assert_eq!(
            ObservedEcuSignal::from_can_frame(&standard_frame(id, &[2, 0])),
            None
        );
        assert_eq!(
            ObservedEcuSignal::from_can_frame(&standard_frame(id, &[1, 1])),
            None
        );
    }
}

#[test]
fn observed_signal_decoder_requires_exactly_two_payload_bytes() {
    for id in [ID_HAZARD, ID_LEFT_TURN_REQUEST, ID_RIGHT_TURN_REQUEST] {
        for payload in [&[][..], &[1][..], &[1, 0, 0][..]] {
            assert_eq!(
                ObservedEcuSignal::from_can_frame(&standard_frame(id, payload)),
                None
            );
        }
    }
}

#[test]
fn observed_signal_decoder_rejects_extended_ids() {
    for id in [ID_HAZARD, ID_LEFT_TURN_REQUEST, ID_RIGHT_TURN_REQUEST] {
        let frame = CanFrame::new(
            ExtendedId::new(id as u32).expect("extended CAN id"),
            &[1, 0],
        )
        .expect("CAN frame");
        assert_eq!(ObservedEcuSignal::from_can_frame(&frame), None);
    }
}
