//! Contract tests for the strict Digital Twin CAN `0x105` hazard-button codec.

use crate::signals::{ControlSignal, ID_HAZARD};
use socketcan::{CanFrame, EmbeddedFrame, ExtendedId, StandardId};

fn standard_frame(id: u16, data: &[u8]) -> CanFrame {
    CanFrame::new(StandardId::new(id).expect("standard CAN id"), data).expect("CAN frame")
}

#[test]
fn hazard_button_encodes_to_exact_can_contract() {
    for (pressed, expected) in [(false, [0, 0]), (true, [1, 0])] {
        let frame = ControlSignal::HazardButton(pressed)
            .to_can_frame()
            .expect("encode hazard button");

        assert_eq!(frame.id(), StandardId::new(ID_HAZARD).unwrap().into());
        assert_eq!(frame.data(), expected);
    }
}

#[test]
fn hazard_button_round_trips() {
    for signal in [
        ControlSignal::HazardButton(false),
        ControlSignal::HazardButton(true),
    ] {
        let frame = signal.to_can_frame().expect("encode hazard button");
        assert_eq!(ControlSignal::from_can_frame(&frame), Some(signal));
    }
}

#[test]
fn hazard_decoder_rejects_wrong_standard_id() {
    let frame = standard_frame(ID_HAZARD - 1, &[1, 0]);
    assert_eq!(ControlSignal::from_can_frame(&frame), None);
}

#[test]
fn hazard_decoder_rejects_extended_id() {
    let frame = CanFrame::new(
        ExtendedId::new(ID_HAZARD as u32).expect("extended CAN id"),
        &[1, 0],
    )
    .expect("CAN frame");

    assert_eq!(ControlSignal::from_can_frame(&frame), None);
}

#[test]
fn hazard_decoder_rejects_wrong_payload_lengths() {
    for payload in [&[][..], &[1][..], &[1, 0, 0][..]] {
        let frame = standard_frame(ID_HAZARD, payload);
        assert_eq!(ControlSignal::from_can_frame(&frame), None);
    }
}

#[test]
fn hazard_decoder_rejects_unknown_value() {
    let frame = standard_frame(ID_HAZARD, &[2, 0]);
    assert_eq!(ControlSignal::from_can_frame(&frame), None);
}

#[test]
fn hazard_decoder_rejects_nonzero_reserved_byte() {
    let frame = standard_frame(ID_HAZARD, &[1, 1]);
    assert_eq!(ControlSignal::from_can_frame(&frame), None);
}
