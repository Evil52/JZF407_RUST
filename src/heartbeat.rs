//! LED3 proof-of-life blink. No dependencies: if this LED freezes, the
//! executor itself is wedged (panic/deadlock), not just the network.

use embassy_stm32::gpio::Output;
use embassy_time::{Duration, Timer};

#[embassy_executor::task]
pub async fn heartbeat_led_task(mut led3: Output<'static>) {
    loop {
        led3.set_low(); // active-LOW: on
        Timer::after(Duration::from_millis(100)).await;
        led3.set_high();
        Timer::after(Duration::from_millis(5000)).await;
    }
}
