use remotivelabs_broker::generated::base::signal::Payload;

/// Decode only the broker encodings admitted by the observed boolean-signal contract.
pub fn decode_boolean(payload: Option<&Payload>) -> Option<bool> {
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

/// Decode FLCM DBC Ok/Fail: `0`/`"Ok"` → true (Ok), `1`/`"Fail"` → false (Fail).
pub fn decode_ok_fail(payload: Option<&Payload>) -> Option<bool> {
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
