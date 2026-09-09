use remotivelabs_broker::generated::base::signal::Payload;

/// Decode only the broker encodings admitted by the Phase I hazard contract.
pub fn decode_hazard(payload: Option<&Payload>) -> Option<bool> {
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
