//! Shared LED state protected by a Mutex. Active-LOW: High = off, Low = on.
//! Relay is owned exclusively by buttons_task and is not in this struct.

use embassy_stm32::gpio::Output;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};

pub enum LedId { Led1, Led2 }

pub struct OutputPins {
    pub led1: Output<'static>,
    pub led2: Output<'static>,
}

pub struct SharedOutputs {
    inner: Mutex<CriticalSectionRawMutex, Option<OutputPins>>,
}

impl SharedOutputs {
    pub const fn new() -> Self {
        Self { inner: Mutex::new(None) }
    }

    pub fn register(&self, pins: OutputPins) {
        let _ = self.inner.try_lock().map(|mut g| *g = Some(pins));
    }

    pub async fn set_led1(&self, on: bool) {
        let mut g = self.inner.lock().await;
        if let Some(ref mut p) = *g {
            if on { p.led1.set_low() } else { p.led1.set_high() }
        }
    }

    pub async fn set_led2(&self, on: bool) {
        let mut g = self.inner.lock().await;
        if let Some(ref mut p) = *g {
            if on { p.led2.set_low() } else { p.led2.set_high() }
        }
    }

    pub async fn set_all_leds(&self, on: bool) {
        let mut g = self.inner.lock().await;
        if let Some(ref mut p) = *g {
            if on { p.led1.set_low();  p.led2.set_low();  }
            else  { p.led1.set_high(); p.led2.set_high(); }
        }
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
}
