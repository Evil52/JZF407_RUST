//! LED3 (PE15, active-LOW) proof-of-life blink: 100 ms on / 100 ms off (~5 Hz).
//!
//! Runs on its own task with no network or peripheral dependencies, so a steady
//! blink confirms the Embassy executor itself is still scheduling — the key
//! signal we relied on while debugging ETH-without-debugger. If this LED freezes,
//! the executor is wedged (panic/deadlock), not just the network.

use embassy_stm32::gpio::Output;
use embassy_time::{Duration, Timer};

#[embassy_executor::task]
pub async fn heartbeat_led_task(mut led3: Output<'static>) {
    loop {
        led3.set_low(); // active-LOW: drive low = LED on
        Timer::after(Duration::from_millis(100)).await;
        led3.set_high(); // LED off
        Timer::after(Duration::from_millis(5000)).await;
    }
}
