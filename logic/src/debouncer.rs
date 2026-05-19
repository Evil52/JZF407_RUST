//! Async debouncer: stable for N consecutive samples spaced PERIOD apart.
//! Fully unit-testable without embassy — the timing is injected via a trait.

pub const SAMPLE_PERIOD_MS: u64 = 10;
pub const STABLE_COUNT:      u8  = 4; // 4 × 10ms = 40ms stable

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
        let all_ones  = self.history & 0x0F == 0x0F;
        let all_zeros = self.history & 0x0F == 0x00;

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
