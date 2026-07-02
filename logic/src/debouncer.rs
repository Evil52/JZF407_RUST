//! Async debouncer: stable for N consecutive samples spaced PERIOD apart.
//! Fully unit-testable without embassy — the timing is injected via a trait.

pub const SAMPLE_PERIOD_MS: u64 = 10;
pub const STABLE_COUNT:      u8  = 4; // 4 × 10ms = 40ms stable
const STABLE_MASK: u8 = ((1u16 << STABLE_COUNT) - 1) as u8;
const _: () = assert!(STABLE_COUNT > 0 && STABLE_COUNT <= 8);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Edge {
    Rising,
    Falling,
}

/// Pure-logic debouncer state machine.
pub struct Debouncer {
    history: u8,   // shift register of last N raw samples
    stable:  bool, // last debounced state
}

impl Debouncer {
    pub const fn new(initial: bool) -> Self {
        Self {
            history: if initial { 0xFF } else { 0x00 },
            stable:  initial,
        }
    }

    /// Feed one raw sample. Returns `Some(Edge)` on state change.
    pub fn update(&mut self, raw: bool) -> Option<Edge> {
        self.history = (self.history << 1) | (raw as u8);
        let window = self.history & STABLE_MASK;
        let all_ones  = window == STABLE_MASK;
        let all_zeros = window == 0;

        if all_ones && !self.stable {
            self.stable = true;
            return Some(Edge::Rising);
        }
        if all_zeros && self.stable {
            self.stable = false;
            return Some(Edge::Falling);
        }
        None
    }

    pub fn state(&self) -> bool {
        self.stable
    }
}
