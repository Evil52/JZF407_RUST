//! Parse incoming MQTT topic+payload into an output command.
//! Pure logic — no embassy/hardware deps — fully unit-testable natively.

#[derive(Debug, PartialEq)]
pub enum OutputCmd {
    Led1(bool),
    Led2(bool),
    AllLeds(bool),
    Relay(bool),
    Ping,
    Reboot,
    Unknown,
}

/// Returns `Some(cmd)` for known topics, `None` for unsubscribed topics.
pub fn dispatch(topic: &str, payload: &[u8]) -> Option<OutputCmd> {
    let on = parse_bool(payload)?;
    match topic {
        "stm32/led/1"     => Some(OutputCmd::Led1(on)),
        "stm32/led/2"     => Some(OutputCmd::Led2(on)),
        "stm32/led/all"   => Some(OutputCmd::AllLeds(on)),
        "stm32/relay"     => Some(OutputCmd::Relay(on)),
        _                 => None,
    }
}

/// Dispatch for topics that don't carry a bool payload.
pub fn dispatch_special(topic: &str) -> Option<OutputCmd> {
    match topic {
        "stm32/ping"          => Some(OutputCmd::Ping),
        "stm32/cmd/reboot"    => Some(OutputCmd::Reboot),
        _                     => None,
    }
}

fn parse_bool(payload: &[u8]) -> Option<bool> {
    match payload {
        b"1" | b"on"  | b"ON"  | b"true"  => Some(true),
        b"0" | b"off" | b"OFF" | b"false" => Some(false),
        _                                   => None,
    }
}
