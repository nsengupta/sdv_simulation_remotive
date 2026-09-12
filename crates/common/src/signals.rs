use socketcan::{CanFrame, EmbeddedFrame, StandardId};

pub const ID_LIFECYCLE: u16 = 0x100;
pub const ID_SPEED: u16 = 0x101;
pub const ID_RPM: u16 = 0x102;
pub const ID_AMBIENT_LUX: u16 = 0x103;
/// Binary rain-presence signal from the windshield rain sensor.
/// `true` = rain detected; `false` = no rain.
pub const ID_RAIN_DETECTED: u16 = 0x104;
pub const ID_HAZARD: u16 = 0x105;
pub const ID_LEFT_TURN_REQUEST: u16 = 0x106;
pub const ID_RIGHT_TURN_REQUEST: u16 = 0x107;

/// A lifecycle request decoded from an external ingress carrier.
///
/// Lifecycle is deliberately separate from [`VssSignal`]: powering the twin on or off is a
/// command, not vehicle telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleCommand {
    PowerOn,
    PowerOff,
}

impl LifecycleCommand {
    /// Decode the strict eight-byte lifecycle contract carried on standard CAN ID `0x100`.
    pub fn from_can_frame(frame: &CanFrame) -> Option<Self> {
        let socketcan::Id::Standard(id) = frame.id() else {
            return None;
        };
        if id.as_raw() != ID_LIFECYCLE {
            return None;
        }

        let data = frame.data();
        if data.len() != 8 || data[1..].iter().any(|byte| *byte != 0) {
            return None;
        }

        match data[0] {
            0 => Some(Self::PowerOff),
            1 => Some(Self::PowerOn),
            _ => None,
        }
    }

    /// Encode this lifecycle request as the strict eight-byte CAN `0x100` contract.
    pub fn to_can_frame(&self) -> Result<CanFrame, socketcan::Error> {
        let id = StandardId::new(ID_LIFECYCLE).expect("lifecycle CAN ID is a valid standard ID");
        let opcode = match self {
            Self::PowerOn => 1,
            Self::PowerOff => 0,
        };
        CanFrame::new(id, &[opcode, 0, 0, 0, 0, 0, 0, 0]).ok_or_else(|| {
            socketcan::Error::from(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "lifecycle payload must fit a classic CAN frame",
            ))
        })
    }
}

/// A validated driver control received from an external ingress carrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlSignal {
    HazardButton(bool),
}

impl ControlSignal {
    /// Decode the strict two-byte hazard-button contract carried on standard CAN ID `0x105`.
    pub fn from_can_frame(frame: &CanFrame) -> Option<Self> {
        let socketcan::Id::Standard(id) = frame.id() else {
            return None;
        };
        if id.as_raw() != ID_HAZARD {
            return None;
        }

        match frame.data() {
            [0, 0] => Some(Self::HazardButton(false)),
            [1, 0] => Some(Self::HazardButton(true)),
            _ => None,
        }
    }

    /// Encode this control as the strict two-byte CAN `0x105` contract.
    pub fn to_can_frame(&self) -> Result<CanFrame, socketcan::Error> {
        let id = StandardId::new(ID_HAZARD).expect("hazard CAN ID is a valid standard ID");
        let Self::HazardButton(pressed) = self;
        CanFrame::new(id, &[*pressed as u8, 0]).ok_or_else(|| {
            socketcan::Error::from(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "hazard payload must fit a classic CAN frame",
            ))
        })
    }
}

/// A validated, transport-independent observation received from an ECU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedEcuSignal {
    HazardButton(bool),
    LeftTurnRequest(bool),
    RightTurnRequest(bool),
}

impl ObservedEcuSignal {
    /// Decode one strict two-byte boolean observation from a standard internal CAN carrier.
    pub fn from_can_frame(frame: &CanFrame) -> Option<Self> {
        let socketcan::Id::Standard(id) = frame.id() else {
            return None;
        };
        let value = match frame.data() {
            [0, 0] => false,
            [1, 0] => true,
            _ => return None,
        };

        match id.as_raw() {
            ID_HAZARD => Some(Self::HazardButton(value)),
            ID_LEFT_TURN_REQUEST => Some(Self::LeftTurnRequest(value)),
            ID_RIGHT_TURN_REQUEST => Some(Self::RightTurnRequest(value)),
            _ => None,
        }
    }

    /// Encode one observation as its strict two-byte standard internal CAN carrier.
    pub fn to_can_frame(self) -> Result<CanFrame, socketcan::Error> {
        let (id, value) = match self {
            Self::HazardButton(value) => (ID_HAZARD, value),
            Self::LeftTurnRequest(value) => (ID_LEFT_TURN_REQUEST, value),
            Self::RightTurnRequest(value) => (ID_RIGHT_TURN_REQUEST, value),
        };
        let id = StandardId::new(id).expect("observed ECU CAN ID is a valid standard ID");
        CanFrame::new(id, &[value as u8, 0]).ok_or_else(|| {
            socketcan::Error::from(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "observed ECU payload must fit a classic CAN frame",
            ))
        })
    }
}

/// An interpreted Vehicle Signal Specification value.
///
/// Today these values are decoded from CAN frames. Future KUKSA integration will interpret
/// blueprint paths and values into this same semantic vocabulary before twin ingress. Lifecycle,
/// control requests, and actuator feedback do not belong in this enum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VssSignal {
    /// Vehicle.Speed (Unit: km/h, Scaling: 0.01). Decoded for future observed-speed ECUs; twin derives speed from RPM today.
    Speed(f64),
    /// Vehicle.Powertrain.CombustionEngine.Speed (Unit: rpm, Scaling: 1.0)
    EngineRpm(u16),
    /// Vehicle.Cabin or exterior ambient light sensor (Unit: lux, Scaling: 1.0)
    AmbientLux(u16),
    /// Vehicle.Body.Windshield.Front.WipingSystem.RainSensor — binary detection.
    /// `true` = rain present; `false` = rain absent.
    RainDetected(bool),
}

impl VssSignal {
    /// Decode a raw CAN Frame into a VSS Signal
    pub fn from_can_frame(frame: &CanFrame) -> Option<Self> {
        // Only standard (11-bit) IDs are supported here; extended / FD-only shapes are rejected.
        // For a standard frame, `as_raw` is the numeric ID (0..=0x7FF).
        let id = match frame.id() {
            socketcan::Id::Standard(s) => s.as_raw(),
            _ => return None,
        };

        let data = frame.data();
        if data.len() < 2 {
            return None;
        }

        match id {
            ID_SPEED => {
                let raw = u16::from_be_bytes([data[0], data[1]]);
                Some(Self::Speed(raw as f64 / 100.0))
            }
            ID_RPM => {
                let raw = u16::from_be_bytes([data[0], data[1]]);
                Some(Self::EngineRpm(raw))
            }
            ID_AMBIENT_LUX => {
                let raw = u16::from_be_bytes([data[0], data[1]]);
                Some(Self::AmbientLux(raw))
            }
            ID_RAIN_DETECTED => Some(Self::RainDetected(data[0] != 0)),
            _ => None,
        }
    }

    /// Encode a VSS Signal into a raw CAN Frame
    pub fn to_can_frame(&self) -> Result<CanFrame, socketcan::Error> {
        // 1. Helper for safe ID creation (`socketcan::StandardId`).
        // `StandardId::new` accepts only values valid for an 11-bit standard CAN ID; otherwise `None`.
        let can_standard_id = |id: u16| {
            StandardId::new(id).ok_or_else(|| {
                socketcan::Error::from(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Invalid CAN ID: {:#X}", id),
                ))
            })
        };

        // 2. Helper for safe Frame creation
        let build_frame = |id: u16, data: &[u8]| {
            let cid = can_standard_id(id)?;
            CanFrame::new(cid, data).ok_or_else(|| {
                socketcan::Error::from(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Data length exceeds CAN standard (8 bytes)",
                ))
            })
        };

        match self {
            Self::Speed(val) => {
                let scaled = (val * 100.0) as u16;
                build_frame(ID_SPEED, &scaled.to_be_bytes())
            }
            Self::EngineRpm(val) => build_frame(ID_RPM, &val.to_be_bytes()),
            Self::AmbientLux(val) => build_frame(ID_AMBIENT_LUX, &val.to_be_bytes()),
            Self::RainDetected(val) => {
                // Byte 0: 0x01 = rain, 0x00 = no rain. Byte 1: reserved zero.
                build_frame(ID_RAIN_DETECTED, &[*val as u8, 0])
            }
        }
    }
}
