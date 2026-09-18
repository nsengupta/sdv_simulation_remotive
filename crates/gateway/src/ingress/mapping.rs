use common::facade::{
    ControlSignal, LifecycleCommand, ObservedEcuSignal, TwinIngressEvent, VssSignal,
};
use socketcan::CanFrame;

/// Decode a generic CAN frame into the canonical, transport-independent twin ingress vocabulary.
///
/// Device-specific actuator response frames are intentionally handled by their correlation-aware
/// policies after this generic lifecycle/telemetry decoder returns `None`.
pub fn can_frame_to_twin_ingress(frame: &CanFrame) -> Option<TwinIngressEvent> {
    if let Some(command) = LifecycleCommand::from_can_frame(frame) {
        return Some(TwinIngressEvent::Lifecycle(command));
    }

    if let Some(observed) = ObservedEcuSignal::from_can_frame(frame) {
        return Some(TwinIngressEvent::ObservedEcu(observed));
    }

    if let Some(control) = ControlSignal::from_can_frame(frame) {
        return Some(TwinIngressEvent::Control(control));
    }

    match VssSignal::from_can_frame(frame) {
        Some(VssSignal::EngineRpm(rpm)) => {
            Some(TwinIngressEvent::Telemetry(VssSignal::EngineRpm(rpm)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::can_frame_to_twin_ingress;
    use common::facade::{
        ControlSignal, LifecycleCommand, ObservedEcuSignal, TwinIngressEvent, VssSignal,
    };
    use socketcan::{CanFrame, EmbeddedFrame, StandardId};

    #[test]
    fn lifecycle_power_on_frame_maps_to_twin_ingress() {
        let frame = LifecycleCommand::PowerOn
            .to_can_frame()
            .expect("encode PowerOn");

        assert!(matches!(
            can_frame_to_twin_ingress(&frame),
            Some(TwinIngressEvent::Lifecycle(LifecycleCommand::PowerOn))
        ));
    }

    #[test]
    fn lifecycle_power_off_frame_maps_to_twin_ingress() {
        let frame = LifecycleCommand::PowerOff
            .to_can_frame()
            .expect("encode PowerOff");

        assert!(matches!(
            can_frame_to_twin_ingress(&frame),
            Some(TwinIngressEvent::Lifecycle(LifecycleCommand::PowerOff))
        ));
    }

    #[test]
    fn engine_rpm_frame_maps_to_twin_telemetry() {
        let frame = VssSignal::EngineRpm(4567)
            .to_can_frame()
            .expect("encode RPM");

        assert!(matches!(
            can_frame_to_twin_ingress(&frame),
            Some(TwinIngressEvent::Telemetry(VssSignal::EngineRpm(4567)))
        ));
    }

    #[test]
    fn observed_hazard_button_frame_maps_to_twin_observed_ecu() {
        let frame = ObservedEcuSignal::HazardButton(true)
            .to_can_frame()
            .expect("encode observed hazard");

        assert!(matches!(
            can_frame_to_twin_ingress(&frame),
            Some(TwinIngressEvent::ObservedEcu(
                ObservedEcuSignal::HazardButton(true)
            ))
        ));
        assert!(!matches!(
            can_frame_to_twin_ingress(&frame),
            Some(TwinIngressEvent::Control(ControlSignal::HazardButton(_)))
        ));
    }

    #[test]
    fn observed_left_turn_request_frame_maps_to_twin_observed_ecu() {
        let frame = ObservedEcuSignal::LeftTurnRequest(true)
            .to_can_frame()
            .expect("encode observed left request");

        assert!(matches!(
            can_frame_to_twin_ingress(&frame),
            Some(TwinIngressEvent::ObservedEcu(
                ObservedEcuSignal::LeftTurnRequest(true)
            ))
        ));
    }

    #[test]
    fn observed_right_turn_request_frame_maps_to_twin_observed_ecu() {
        let frame = ObservedEcuSignal::RightTurnRequest(false)
            .to_can_frame()
            .expect("encode observed right request");

        assert!(matches!(
            can_frame_to_twin_ingress(&frame),
            Some(TwinIngressEvent::ObservedEcu(
                ObservedEcuSignal::RightTurnRequest(false)
            ))
        ));
    }

    #[test]
    fn observed_left_low_beam_status_frame_maps_to_twin_observed_ecu() {
        let frame = ObservedEcuSignal::LeftLowBeamStatus(true)
            .to_can_frame()
            .expect("encode observed left low-beam status");

        assert!(matches!(
            can_frame_to_twin_ingress(&frame),
            Some(TwinIngressEvent::ObservedEcu(
                ObservedEcuSignal::LeftLowBeamStatus(true)
            ))
        ));
    }

    #[test]
    fn observed_right_low_beam_status_frame_maps_to_twin_observed_ecu() {
        let frame = ObservedEcuSignal::RightLowBeamStatus(false)
            .to_can_frame()
            .expect("encode observed right low-beam status");

        assert!(matches!(
            can_frame_to_twin_ingress(&frame),
            Some(TwinIngressEvent::ObservedEcu(
                ObservedEcuSignal::RightLowBeamStatus(false)
            ))
        ));
    }

    #[test]
    fn malformed_observed_ids_and_dlcs_are_not_twin_ingress() {
        let unknown = CanFrame::new(StandardId::new(0x7ff).unwrap(), &[1, 0]).unwrap();
        assert!(can_frame_to_twin_ingress(&unknown).is_none());

        for id in [0x105, 0x106, 0x107, 0x108, 0x109] {
            for data in [&[][..], &[1][..], &[1, 0, 0][..], &[2, 0][..], &[1, 1][..]] {
                let frame = CanFrame::new(StandardId::new(id).unwrap(), data).unwrap();
                assert!(
                    can_frame_to_twin_ingress(&frame).is_none(),
                    "id={id:#x} data={data:?} must not decode"
                );
            }
        }
    }

    #[test]
    fn unsupported_observation_ingress_telemetry_is_not_twin_ingress() {
        for signal in [
            VssSignal::Speed(50.0),
            VssSignal::AmbientLux(28),
            VssSignal::RainDetected(true),
        ] {
            let frame = signal.to_can_frame().expect("encode legacy VSS signal");
            assert!(can_frame_to_twin_ingress(&frame).is_none());
        }
    }

    #[test]
    fn unknown_frame_is_not_twin_ingress() {
        let frame = CanFrame::new(StandardId::new(0x7ff).unwrap(), &[0; 8]).unwrap();
        assert!(can_frame_to_twin_ingress(&frame).is_none());
    }
}
