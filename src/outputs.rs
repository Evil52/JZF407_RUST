//! Shared GPIO outputs (LED1, LED2, relay) behind a single mutex so that
//! buttons_task, mqtt_task and web_task can all drive them without owning the pins.
//!
//! Polarity differs per output and is the #1 source of confusion when debugging:
//!   - LEDs  are active-LOW  → on = set_low(),  off = set_high()
//!   - Relay is active-HIGH → on = set_high(), off = set_low()
//!
//! All accessors use `try_lock()` (never block): a momentarily-contended output
//! is simply skipped rather than stalling an async task in a critical section.

use embassy_stm32::gpio::Output;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};

pub enum LedId { Led1, Led2 }

pub struct OutputPins {
    pub led1: Output<'static>,
    pub led2: Output<'static>,
    pub relay: Output<'static>,
}

pub struct SharedOutputs {
    inner: Mutex<CriticalSectionRawMutex, Option<OutputPins>>,
}

impl Default for SharedOutputs {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedOutputs {
    pub const fn new() -> Self {
        Self { inner: Mutex::new(None) }
    }

    pub fn register(&self, pins: OutputPins) {
        let _ = self.inner.try_lock().map(|mut g| *g = Some(pins));
    }

    pub fn set(&self, led: LedId, on: bool) {
        if let Ok(mut g) = self.inner.try_lock() {
            if let Some(ref mut p) = *g {
                match led {
                    LedId::Led1 => if on { p.led1.set_low() } else { p.led1.set_high() },
                    LedId::Led2 => if on { p.led2.set_low() } else { p.led2.set_high() },
                }
            }
        }
    }

    /// Relay is active-HIGH: on = drive High, off = drive Low (inverted vs. LEDs).
    pub fn set_relay(&self, on: bool) {
        if let Ok(mut g) = self.inner.try_lock() {
            if let Some(ref mut p) = *g {
                if on { p.relay.set_high() } else { p.relay.set_low() }
            }
        }
    }

    /// Read actual relay pin state (active-HIGH: High = ON).
    pub fn get_relay(&self) -> bool {
        self.inner.try_lock().ok()
            .and_then(|g| g.as_ref().map(|p| p.relay.is_set_high()))
            .unwrap_or(false)
    }
}
