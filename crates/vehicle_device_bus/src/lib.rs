//! CAN wire protocols for vehicle actuators and devices.
//!
//! One module per device under [`devices`] (e.g. [`devices::front_headlamp`]).
//! Gateway and standalone actuator binaries depend on this crate; domain logic stays in [`common`].

pub mod can;
pub mod devices;

/// Default SocketCAN interface shared by Gateway, emulator, Remotive bridge, and actuators.
pub const DEFAULT_CAN_INTERFACE: &str = "vcan0";

#[cfg(test)]
mod tests {
    #[test]
    fn default_can_interface_is_vcan0() {
        assert_eq!(crate::DEFAULT_CAN_INTERFACE, "vcan0");
    }
}
