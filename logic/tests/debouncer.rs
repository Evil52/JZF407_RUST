// Native unit tests for the debouncer — no embedded dependencies.
// Run with: cargo test --test debouncer

use jzf407_logic::debouncer::{Debouncer, Edge};

#[test]
fn idle_produces_no_edge() {
    let mut d = Debouncer::new(false);
    for _ in 0..16 {
        assert_eq!(d.update(false), None);
    }
}

#[test]
fn single_glitch_suppressed() {
    let mut d = Debouncer::new(false);
    // One high sample then back to low — must not trigger
    d.update(true);
    assert_eq!(d.update(false), None);
    assert_eq!(d.state(), false);
}

#[test]
fn stable_high_triggers_rising() {
    let mut d = Debouncer::new(false);
    // Need 4 consecutive highs
    assert_eq!(d.update(true), None);
    assert_eq!(d.update(true), None);
    assert_eq!(d.update(true), None);
    let edge = d.update(true);
    assert_eq!(edge, Some(Edge::Rising));
    assert_eq!(d.state(), true);
}

#[test]
fn stable_low_triggers_falling() {
    let mut d = Debouncer::new(true);
    assert_eq!(d.update(false), None);
    assert_eq!(d.update(false), None);
    assert_eq!(d.update(false), None);
    let edge = d.update(false);
    assert_eq!(edge, Some(Edge::Falling));
    assert_eq!(d.state(), false);
}

#[test]
fn rising_then_falling() {
    let mut d = Debouncer::new(false);
    for _ in 0..4 { d.update(true); }
    assert_eq!(d.state(), true);
    for _ in 0..3 { d.update(false); }
    assert_eq!(d.state(), true); // not yet
    let e = d.update(false);
    assert_eq!(e, Some(Edge::Falling));
}

#[test]
fn noisy_signal_no_false_edge() {
    let mut d = Debouncer::new(false);
    let noisy = [true, false, true, false, true, false, true, false];
    let mut edges = 0usize;
    for &s in &noisy {
        if d.update(s).is_some() { edges += 1; }
    }
    assert_eq!(edges, 0);
}

#[test]
fn no_double_edge_on_same_stable_state() {
    let mut d = Debouncer::new(false);
    for _ in 0..4 { d.update(true); }
    // Additional highs should not re-trigger
    assert_eq!(d.update(true), None);
    assert_eq!(d.update(true), None);
}

#[test]
fn initial_state_low() {
    let d = Debouncer::new(false);
    assert_eq!(d.state(), false);
}

#[test]
fn initial_state_high() {
    let d = Debouncer::new(true);
    assert_eq!(d.state(), true);
}

#[test]
fn three_highs_not_enough() {
    let mut d = Debouncer::new(false);
    for _ in 0..3 { assert_eq!(d.update(true), None); }
    assert_eq!(d.state(), false);
}

#[test]
fn glitch_between_stable_transitions() {
    let mut d = Debouncer::new(false);
    // Almost stable: 3 highs, 1 low glitch, then stable highs
    d.update(true); d.update(true); d.update(true);
    d.update(false); // glitch resets count
    // Now need 4 more consecutive highs
    for _ in 0..3 { assert_eq!(d.update(true), None); }
    let e = d.update(true);
    assert_eq!(e, Some(Edge::Rising));
}
