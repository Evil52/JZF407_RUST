//! AT24C02 access via embassy-stm32 blocking I2C1.
//! Device address 0x50 (A0/A1/A2 tied to GND on JZ-F407).

use embassy_stm32::{
    i2c::{I2c, Master},
    mode::Blocking,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};

pub type ChipI2c = I2c<'static, Blocking, Master>;

const EEPROM_ADDR: u8 = 0x50;
const WRITE_CYCLE_MS: u64 = 5;

/// Global EEPROM handle — initialized once in main, used by persistence + web.
pub static EEPROM: Mutex<CriticalSectionRawMutex, Option<Eeprom>> = Mutex::new(None);

pub struct Eeprom {
    i2c: ChipI2c,
}

impl Eeprom {
    pub fn new(i2c: ChipI2c) -> Self {
        Self { i2c }
    }

    /// Store the EEPROM instance in the global mutex.
    pub fn init_global(self) {
        cortex_m::interrupt::free(|_| {
            *EEPROM.try_lock().unwrap() = Some(self);
        });
    }

    /// Sequential read: set address pointer then read `buf.len()` bytes.
    /// Blocking call — short enough not to starve the executor.
    pub fn read(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), embassy_stm32::i2c::Error> {
        self.i2c.blocking_write_read(EEPROM_ADDR, &[addr], buf)
    }

    /// Write one byte per transaction with 5ms write-cycle delay between each.
    pub async fn write_bytes(
        &mut self,
        start_addr: u8,
        data: &[u8],
    ) -> Result<(), embassy_stm32::i2c::Error> {
        for (i, &byte) in data.iter().enumerate() {
            let mem_addr = start_addr.wrapping_add(i as u8);
            self.i2c.blocking_write(EEPROM_ADDR, &[mem_addr, byte])?;
            embassy_time::Timer::after(embassy_time::Duration::from_millis(WRITE_CYCLE_MS)).await;
        }
        Ok(())
    }
}
