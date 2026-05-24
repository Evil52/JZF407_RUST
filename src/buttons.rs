//! S1 (PE10) / S2 (PE11) button polling with async debounce.
//! Relay control is disabled — relay is driven exclusively via MQTT (stm32/relay).
//! Buttons have hardware pull-ups (R17/R18): idle = High, pressed = Low.

use embassy_stm32::{
    gpio::{Input, Pull},
    Peri,
    peripherals,
};
use embassy_time::{Duration, Timer};
use jzf407_logic::debouncer::{Debouncer, SAMPLE_PERIOD_MS};

#[embassy_executor::task]
pub async fn buttons_task(
    s1_pin: Peri<'static, peripherals::PE10>,
    s2_pin: Peri<'static, peripherals::PE11>,
) {
    let s1 = Input::new(s1_pin, Pull::Up);
    let s2 = Input::new(s2_pin, Pull::Up);

    let mut deb_s1 = Debouncer::new(false);
    let mut deb_s2 = Debouncer::new(false);

    loop {
        Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;

        // S1 / S2 relay control commented out — relay driven by MQTT only.
        // if let Some(Edge::Rising) = deb_s1.update(s1.is_low()) {
        //     crate::outputs::pulse_relay();
        //     defmt::info!("S1: relay pulse");
        //     crate::mqtt::RELAY_CHANGE.signal(true);
        // }
        // if let Some(Edge::Rising) = deb_s2.update(s2.is_low()) {
        //     crate::outputs::relay_off();
        //     defmt::info!("S2: relay off");
        //     crate::mqtt::RELAY_CHANGE.signal(false);
        // }

        // Keep debouncer state updated so re-enabling later works cleanly.
        let _ = deb_s1.update(s1.is_low());
        let _ = deb_s2.update(s2.is_low());
    }
}
