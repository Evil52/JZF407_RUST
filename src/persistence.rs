//! Persist LED state to AT24C02 at offset 0 (network config lives at 16).
//! The relay is a momentary 2 s pulse and is deliberately NOT persisted;
//! byte 4 stays reserved so the layout is stable.
//!
//! Layout (bytes 0..7):
//! ```text
//!   [0..3]  magic 0xCA 0xFE 0xF0 0x0D
//!   [4]     reserved (was relay)
//!   [5]     led1: bit0
//!   [6]     led2: bit0
//!   [7]     reserved
//! ```

use crate::eeprom;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

const MAGIC: [u8; 4] = [0xCA, 0xFE, 0xF0, 0x0D];
const BASE: u8 = 0;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct OutputState {
    pub led1: bool,
    pub led2: bool,
}

pub static STATE_CACHE: Mutex<CriticalSectionRawMutex, Option<OutputState>> = Mutex::new(None);

pub fn load_state() -> OutputState {
    let mut buf = [0u8; 8];
    let read_ok = eeprom::EEPROM
        .try_lock()
        .ok()
        .and_then(|mut g| g.as_mut().and_then(|ee| ee.read(BASE, &mut buf).ok()))
        .is_some();
    let state = if read_ok && buf[0..4] == MAGIC {
        OutputState {
            led1: buf[5] & 1 != 0,
            led2: buf[6] & 1 != 0,
        }
    } else {
        OutputState::default()
    };

    cortex_m::interrupt::free(|_| {
        *STATE_CACHE.try_lock().unwrap() = Some(state);
    });

    state
}

pub async fn save_led1(on: bool) {
    save_field(|s| s.led1 = on).await;
}

pub async fn save_led2(on: bool) {
    save_field(|s| s.led2 = on).await;
}

pub async fn save_leds(led1: bool, led2: bool) {
    save_field(|s| {
        s.led1 = led1;
        s.led2 = led2;
    })
    .await;
}

// Cache is updated only after a successful EEPROM write: a failed write leaves
// the old cached value, so the next call with the same state retries instead of
// short-circuiting on an already-"new" cache.
async fn save_field(update: impl FnOnce(&mut OutputState)) {
    let mut cache = STATE_CACHE.lock().await;
    let mut state = (*cache).unwrap_or_default();
    update(&mut state);
    if *cache == Some(state) {
        return;
    }

    let buf: [u8; 8] = [
        MAGIC[0],
        MAGIC[1],
        MAGIC[2],
        MAGIC[3],
        0,
        state.led1 as u8,
        state.led2 as u8,
        0,
    ];

    let mut guard = eeprom::EEPROM.lock().await;
    if let Some(ref mut ee) = *guard {
        if ee.write_bytes(BASE, &buf).await.is_ok() {
            *cache = Some(state);
        }
    }
}
