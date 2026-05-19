//! LED3 heartbeat: 100ms flash every 7s, independent of network.
//! PE15, active-LOW.

use embassy_stm32::gpio::Output;
use embassy_time::{Duration, Timer};

#[embassy_executor::task]
pub async fn heartbeat_led_task(mut led3: Output<'static>) {
    loop {
        // Flash: LOW = on for 100ms, off for 6.9s = 7s cycle
        led3.set_low();
        Timer::after(Duration::from_millis(500)).await;
        led3.set_high();
        Timer::after(Duration::from_millis(3_900)).await;
    }
}
