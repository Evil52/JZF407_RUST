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
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Timer};

#[derive(Clone, Copy)]
pub enum LedId { Led1, Led2 }

/// How long the relay stays energised after a trigger before dropping back to OFF.
pub const RELAY_PULSE: Duration = Duration::from_secs(2);

/// Command sent to `relay_task`. The relay is a monostable pulse output: a
/// `Pulse` drives it HIGH for `RELAY_PULSE`, then it returns to OFF on its own.
/// An explicit `Off` cancels an in-flight pulse immediately.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RelayCmd { Pulse, Off }

/// Trigger channel into `relay_task`. Callers (buttons / mqtt / web) never touch
/// the relay pin directly — they signal a command and `relay_task` owns the
/// timing. `Signal` keeps only the latest command, which is exactly the
/// retrigger-restarts-the-timer semantics we want.
static RELAY_TRIGGER: Signal<CriticalSectionRawMutex, RelayCmd> = Signal::new();

/// Fire a 2 s relay pulse. Returns immediately; the pin is driven by `relay_task`.
/// Re-triggering while a pulse is active restarts the 2 s window.
pub fn pulse_relay() {
    RELAY_TRIGGER.signal(RelayCmd::Pulse);
}

/// Cancel any in-flight pulse and force the relay OFF now.
pub fn relay_off() {
    RELAY_TRIGGER.signal(RelayCmd::Off);
}

/// Owns relay pulse timing. Idle until a `Pulse` arrives, then holds the relay
/// HIGH for `RELAY_PULSE`. A new command (`Pulse` retriggers, `Off` cancels)
/// arriving during the hold preempts the timer via `select`.
#[embassy_executor::task]
pub async fn relay_task(outputs: &'static SharedOutputs) {
    use embassy_futures::select::{select, Either};

    loop {
        // Idle: relay OFF, wait for the first trigger.
        match RELAY_TRIGGER.wait().await {
            RelayCmd::Off => continue, // already off
            RelayCmd::Pulse => {}
        }

        // Energise and hold for the pulse window, restarting on each retrigger.
        loop {
            outputs.set_relay(true);
            match select(Timer::after(RELAY_PULSE), RELAY_TRIGGER.wait()).await {
                // Pulse elapsed with no new trigger → drop and go idle.
                Either::First(()) => break,
                // Retriggered → restart the window.
                Either::Second(RelayCmd::Pulse) => continue,
                // Explicit off mid-pulse → drop immediately.
                Either::Second(RelayCmd::Off) => break,
            }
        }
        outputs.set_relay(false);
    }
}

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

    /// Drive the relay pin (active-HIGH: on = High, off = Low). Internal to the
    /// pulse state machine — callers should use [`pulse_relay`] / [`relay_off`]
    /// rather than touching the pin directly, so timing stays owned by
    /// [`relay_task`].
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

    /// Read actual LED pin state (active-LOW: Low = ON).
    pub fn get_led(&self, led: LedId) -> bool {
        self.inner.try_lock().ok()
            .and_then(|g| g.as_ref().map(|p| match led {
                LedId::Led1 => p.led1.is_set_low(),
                LedId::Led2 => p.led2.is_set_low(),
            }))
            .unwrap_or(false)
    }
}
