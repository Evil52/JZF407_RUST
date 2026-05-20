//! Firmware-side config: EEPROM load on top of the pure-logic NetworkConfig.

use crate::eeprom;
pub use jzf407_logic::config::NetworkConfig;

const BASE: u8 = 16;
const LEN: usize = 49;

/// Read NetworkConfig from AT24C02 starting at byte 16.
/// Returns defaults if the magic is wrong or on any I2C error, so a blank or
/// unreachable EEPROM always boots the device on its built-in defaults.
pub fn load_config() -> NetworkConfig {
    let mut buf = [0u8; LEN];
    let read_ok = crate::eeprom::EEPROM
        .try_lock()
        .ok()
        .and_then(|mut g| g.as_mut().and_then(|ee| ee.read(BASE, &mut buf).ok()))
        .is_some();
    if !read_ok {
        return NetworkConfig::default();
    }
    NetworkConfig::from_bytes(&buf).unwrap_or_default()
}

/// Write NetworkConfig to AT24C02 at byte 16.
pub async fn save_config(cfg: &NetworkConfig) -> Result<(), embassy_stm32::i2c::Error> {
    let mut guard = eeprom::EEPROM.lock().await;
    let ee = guard.as_mut().ok_or(embassy_stm32::i2c::Error::Overrun)?;
    let buf = cfg.to_bytes();
    ee.write_bytes(BASE, &buf).await
}
