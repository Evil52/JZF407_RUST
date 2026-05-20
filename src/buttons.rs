//! S1 (PE10) / S2 (PE11) button polling with async debounce.
//! S1 → relay ON, S2 → relay OFF. Relay PD4, active-HIGH (see OUTPUTS.set_relay).
//! Buttons have hardware pull-ups (R17/R18): idle = High, pressed = Low.

use embassy_stm32::{
    gpio::{Input, Pull},
    Peri,
    peripherals,
};
use embassy_time::{Duration, Timer};
use jzf407_logic::debouncer::{Debouncer, Edge, SAMPLE_PERIOD_MS};

#[embassy_executor::task]
pub async fn buttons_task(
    s1_pin: Peri<'static, peripherals::PE10>,
    s2_pin: Peri<'static, peripherals::PE11>,
) {
    let s1 = Input::new(s1_pin, Pull::Up);
    let s2 = Input::new(s2_pin, Pull::Up);

    // Idle (not pressed) = High → is_low() == false, so seed with false. A press
    // pulls the pin Low → is_low() rises false→true, detected as a Rising edge.
    let mut deb_s1 = Debouncer::new(false);
    let mut deb_s2 = Debouncer::new(false);

    loop {
        Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;

        if let Some(Edge::Rising) = deb_s1.update(s1.is_low()) {
            crate::OUTPUTS.set_relay(true);
            defmt::info!("S1: relay ON");
            crate::mqtt::RELAY_CHANGE.signal(true);
            crate::persistence::save_relay(true).await;
        }
        if let Some(Edge::Rising) = deb_s2.update(s2.is_low()) {
            crate::OUTPUTS.set_relay(false);
            defmt::info!("S2: relay OFF");
            crate::mqtt::RELAY_CHANGE.signal(false);
            crate::persistence::save_relay(false).await;
        }
    }
}
