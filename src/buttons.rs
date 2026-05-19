//! S1 (PE10) / S2 (PE11) button polling with async debounce.
//! S1 → relay ON, S2 → relay OFF. Relay PD4, active-LOW.
//! Buttons have hardware pull-ups (R17/R18): idle = High, pressed = Low.

use embassy_stm32::{
    gpio::{Input, Level, Output, Pull, Speed},
    Peri,
    peripherals,
};
use embassy_time::{Duration, Timer};
use jzf407_logic::debouncer::{Debouncer, Edge, SAMPLE_PERIOD_MS};

#[embassy_executor::task]
pub async fn buttons_task(
    s1_pin: Peri<'static, peripherals::PE10>,
    s2_pin: Peri<'static, peripherals::PE11>,
    relay:  Output<'static>,
) {
    let s1 = Input::new(s1_pin, Pull::Up);
    let s2 = Input::new(s2_pin, Pull::Up);

    let mut deb_s1 = Debouncer::new(true);
    let mut deb_s2 = Debouncer::new(true);
    let mut relay = relay;
    let mut relay_on = false;

    loop {
        Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;

        // pressed = Low → raw = true
        if let Some(Edge::Falling) = deb_s1.update(s1.is_low()) {
            relay_on = true;
            relay.set_low();
            defmt::info!("S1: relay ON");
            crate::mqtt::RELAY_CHANGE.signal(relay_on);
        }
        if let Some(Edge::Falling) = deb_s2.update(s2.is_low()) {
            relay_on = false;
            relay.set_high();
            defmt::info!("S2: relay OFF");
            crate::mqtt::RELAY_CHANGE.signal(relay_on);
        }
    }
}
