//! Shared GPIO outputs. Polarity: LEDs are active-LOW, relay is active-HIGH.
//! All accessors use `try_lock()` (never block) — a momentarily-contended
//! output is skipped rather than stalling a task in a critical section.

use embassy_stm32::gpio::Output;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Timer};

#[derive(Clone, Copy)]
pub enum LedId {
    Led1,
    Led2,
}

pub const RELAY_PULSE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RelayCmd {
    Pulse,
    Off,
}

static RELAY_TRIGGER: Signal<CriticalSectionRawMutex, RelayCmd> = Signal::new();

pub fn pulse_relay() {
    RELAY_TRIGGER.signal(RelayCmd::Pulse);
}

pub fn relay_off() {
    RELAY_TRIGGER.signal(RelayCmd::Off);
}

/// Owns relay pulse timing: `Pulse` holds the pin HIGH for `RELAY_PULSE`
/// (retrigger restarts the window), `Off` cancels. After the pulse ends it
/// signals MQTT with `false` so the broker reflects the actual OFF state.
#[embassy_executor::task]
pub async fn relay_task(outputs: &'static SharedOutputs) {
    use embassy_futures::select::{select, Either};

    loop {
        match RELAY_TRIGGER.wait().await {
            RelayCmd::Off => continue,
            RelayCmd::Pulse => {}
        }

        loop {
            outputs.set_relay(true);
            match select(Timer::after(RELAY_PULSE), RELAY_TRIGGER.wait()).await {
                Either::First(()) => break,
                Either::Second(RelayCmd::Pulse) => continue,
                Either::Second(RelayCmd::Off) => break,
            }
        }
        outputs.set_relay(false);
        crate::mqtt::RELAY_CHANGE.signal(false);
    }
}

pub struct OutputPins {
    pub led1: Output<'static>,
    pub led2: Output<'static>,
    pub relay: Output<'static>,
}

fn drive_active_low(output: &mut Output<'static>, on: bool) {
    if on {
        output.set_low();
    } else {
        output.set_high();
    }
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
        Self {
            inner: Mutex::new(None),
        }
    }

    pub fn register(&self, pins: OutputPins) {
        let _ = self.inner.try_lock().map(|mut g| *g = Some(pins));
    }

    pub fn set(&self, led: LedId, on: bool) {
        let Ok(mut guard) = self.inner.try_lock() else {
            return;
        };
        let Some(pins) = guard.as_mut() else {
            return;
        };
        let output = match led {
            LedId::Led1 => &mut pins.led1,
            LedId::Led2 => &mut pins.led2,
        };
        drive_active_low(output, on);
    }

    /// Callers must use [`pulse_relay`] / [`relay_off`] so timing stays owned
    /// by [`relay_task`].
    pub fn set_relay(&self, on: bool) {
        if let Ok(mut g) = self.inner.try_lock() {
            if let Some(ref mut p) = *g {
                if on {
                    p.relay.set_high()
                } else {
                    p.relay.set_low()
                }
            }
        }
    }

    pub fn get_relay(&self) -> bool {
        self.inner
            .try_lock()
            .ok()
            .and_then(|g| g.as_ref().map(|p| p.relay.is_set_high()))
            .unwrap_or(false)
    }

    pub fn get_led(&self, led: LedId) -> bool {
        self.inner
            .try_lock()
            .ok()
            .and_then(|g| {
                g.as_ref().map(|p| match led {
                    LedId::Led1 => p.led1.is_set_low(),
                    LedId::Led2 => p.led2.is_set_low(),
                })
            })
            .unwrap_or(false)
    }
}
