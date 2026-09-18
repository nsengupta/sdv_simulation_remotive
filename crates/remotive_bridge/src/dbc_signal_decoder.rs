//! Normalize Remotive broker payloads for observed boolean signals.
//!
//! Broker frames arrive as DBC integers (`0`/`1`) or the symbolic labels the
//! DBC assigns to those values (`"Off"`/`"On"`, `"Ok"`/`"Fail"`). This module
//! maps those encodings onto Twin `bool`s so the rest of the bridge can speak
//! one vocabulary.
//!
//! It does **not** decode CAN frames and does **not** parse DBC files. Those
//! stay with the Remotive broker and `vehicle_device_bus`.

use remotivelabs_broker::generated::base::signal::Payload;

/// Decode DBC Off/On polarity: `0`/`"Off"` → `false`, `1`/`"On"` → `true`.
pub fn decode_dbc_off_or_on(payload: Option<&Payload>) -> Option<bool> {
    match payload? {
        Payload::Integer(0) | Payload::Uinteger64(0) => Some(false),
        Payload::Integer(1) | Payload::Uinteger64(1) => Some(true),
        Payload::StrValue(value) if value == "Off" => Some(false),
        Payload::StrValue(value) if value == "On" => Some(true),
        Payload::Double(_)
        | Payload::Arbitration(_)
        | Payload::Empty(_)
        | Payload::Integer(_)
        | Payload::Uinteger64(_)
        | Payload::StrValue(_) => None,
    }
}

/// Decode DBC Ok/Fail polarity: `0`/`"Ok"` → `true` (Ok), `1`/`"Fail"` → `false` (Fail).
pub fn decode_dbc_ok_or_fail(payload: Option<&Payload>) -> Option<bool> {
    match payload? {
        Payload::Integer(0) | Payload::Uinteger64(0) => Some(true),
        Payload::Integer(1) | Payload::Uinteger64(1) => Some(false),
        Payload::StrValue(value) if value == "Ok" => Some(true),
        Payload::StrValue(value) if value == "Fail" => Some(false),
        Payload::Double(_)
        | Payload::Arbitration(_)
        | Payload::Empty(_)
        | Payload::Integer(_)
        | Payload::Uinteger64(_)
        | Payload::StrValue(_) => None,
    }
}
