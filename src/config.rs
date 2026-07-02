use crate::eeprom;
pub use jzf407_logic::config::NetworkConfig;
use jzf407_logic::config::LEN;

const BASE: u8 = 16;

/// Falls back to defaults on wrong magic or any I2C error, so a blank or
/// unreachable EEPROM always boots.
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

pub async fn save_config(cfg: &NetworkConfig) -> Result<(), embassy_stm32::i2c::Error> {
    let mut guard = eeprom::EEPROM.lock().await;
    let ee = guard.as_mut().ok_or(embassy_stm32::i2c::Error::Overrun)?;
    let buf = cfg.to_bytes();
    ee.write_bytes(BASE, &buf).await
}
