//! Persist relay+LED state to AT24C02.
//!
//! Layout (bytes 0..7):
//!   [0..3]  magic 0xCA, 0xFE, 0xF0, 0x0D
//!   [4]     relay:  bit0
//!   [5]     led1:   bit0
//!   [6]     led2:   bit0
//!   [7]     (reserved)
//!
//! RAM-cache: only flush to EEPROM when state actually changes.
//! Uses the global `crate::eeprom::EEPROM` mutex.

use crate::eeprom;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

const MAGIC: [u8; 4] = [0xCA, 0xFE, 0xF0, 0x0D];
const BASE: u8 = 0;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct OutputState {
    pub relay: bool,
    pub led1: bool,
    pub led2: bool,
}

/// RAM-cache guarded by a mutex — shared between buttons_task (saves on change)
/// and mqtt_task (saves on MQTT command).
pub static STATE_CACHE: Mutex<CriticalSectionRawMutex, Option<OutputState>> = Mutex::new(None);

/// Load state from EEPROM (blocking read, once at boot). Returns default if magic is wrong.
pub fn load_state() -> OutputState {
    let mut buf = [0u8; 8];
    let read_ok = eeprom::EEPROM
        .try_lock()
        .ok()
        .and_then(|mut g| g.as_mut().and_then(|ee| ee.read(BASE, &mut buf).ok()))
        .is_some();
    let state = if read_ok && buf[0..4] == MAGIC {
        OutputState {
            relay: buf[4] & 1 != 0,
            led1: buf[5] & 1 != 0,
            led2: buf[6] & 1 != 0,
        }
    } else {
        OutputState::default()
    };

    // Seed the RAM-cache
    cortex_m::interrupt::free(|_| {
        *STATE_CACHE.try_lock().unwrap() = Some(state);
    });

    state
}

pub async fn save_relay(on: bool) {
    save_field(|s| s.relay = on).await;
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

/// Apply a field update to the cached state and flush to EEPROM only if it
/// actually changed. Each caller knows just its own output (relay or LEDs);
/// the cache holds the full state so a partial update never clobbers siblings.
async fn save_field(update: impl FnOnce(&mut OutputState)) {
    let mut cache = STATE_CACHE.lock().await;
    let mut state = (*cache).unwrap_or_default();
    update(&mut state);
    if *cache == Some(state) {
        return;
    }
    *cache = Some(state);
    drop(cache);

    let buf: [u8; 8] = [
        MAGIC[0],
        MAGIC[1],
        MAGIC[2],
        MAGIC[3],
        state.relay as u8,
        state.led1 as u8,
        state.led2 as u8,
        0,
    ];

    let mut guard = eeprom::EEPROM.lock().await;
    if let Some(ref mut ee) = *guard {
        let _ = ee.write_bytes(BASE, &buf).await;
    }
}
