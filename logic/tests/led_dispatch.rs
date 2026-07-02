// Native unit tests for MQTT topic dispatch.
// Run with: cargo test --test led_dispatch

use jzf407_logic::led_dispatch::{dispatch, dispatch_special, OutputCmd};

// ---- led/1 ----

#[test]
fn led1_on() {
    assert_eq!(dispatch("stm32/led/1", b"1"), Some(OutputCmd::Led1(true)));
}
#[test]
fn led1_off() {
    assert_eq!(dispatch("stm32/led/1", b"0"), Some(OutputCmd::Led1(false)));
}
#[test]
fn led1_on_word() {
    assert_eq!(dispatch("stm32/led/1", b"on"), Some(OutputCmd::Led1(true)));
}
#[test]
fn led1_off_word() {
    assert_eq!(
        dispatch("stm32/led/1", b"off"),
        Some(OutputCmd::Led1(false))
    );
}
#[test]
fn led1_true() {
    assert_eq!(
        dispatch("stm32/led/1", b"true"),
        Some(OutputCmd::Led1(true))
    );
}
#[test]
fn led1_false() {
    assert_eq!(
        dispatch("stm32/led/1", b"false"),
        Some(OutputCmd::Led1(false))
    );
}

// ---- led/2 ----

#[test]
fn led2_on() {
    assert_eq!(dispatch("stm32/led/2", b"1"), Some(OutputCmd::Led2(true)));
}
#[test]
fn led2_off() {
    assert_eq!(dispatch("stm32/led/2", b"0"), Some(OutputCmd::Led2(false)));
}

// ---- led/all ----

#[test]
fn all_on() {
    assert_eq!(
        dispatch("stm32/led/all", b"1"),
        Some(OutputCmd::AllLeds(true))
    );
}
#[test]
fn all_off() {
    assert_eq!(
        dispatch("stm32/led/all", b"0"),
        Some(OutputCmd::AllLeds(false))
    );
}

// ---- relay ----

#[test]
fn relay_on() {
    assert_eq!(dispatch("stm32/relay", b"1"), Some(OutputCmd::Relay(true)));
}
#[test]
fn relay_off() {
    assert_eq!(dispatch("stm32/relay", b"0"), Some(OutputCmd::Relay(false)));
}
#[test]
fn relay_on_upper() {
    assert_eq!(dispatch("stm32/relay", b"ON"), Some(OutputCmd::Relay(true)));
}

// ---- unknown/invalid ----

#[test]
fn unknown_topic() {
    assert_eq!(dispatch("stm32/unknown", b"1"), None);
}
#[test]
fn invalid_payload() {
    assert_eq!(dispatch("stm32/led/1", b"maybe"), None);
}
#[test]
fn empty_payload() {
    assert_eq!(dispatch("stm32/led/1", b""), None);
}
#[test]
fn wrong_case_true() {
    assert_eq!(dispatch("stm32/led/1", b"True"), None);
}

// ---- special topics ----

#[test]
fn ping() {
    assert_eq!(dispatch_special("stm32/ping"), Some(OutputCmd::Ping));
}
#[test]
fn reboot() {
    assert_eq!(
        dispatch_special("stm32/cmd/reboot"),
        Some(OutputCmd::Reboot)
    );
}
#[test]
fn special_unknown() {
    assert_eq!(dispatch_special("stm32/led/1"), None);
}
