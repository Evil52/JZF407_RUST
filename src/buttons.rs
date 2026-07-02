// DISABLED: not spawned from main. S1 (PE10) / S2 (PE11) are read and
// debounced by this template but drive nothing yet. To enable, uncomment and
// spawn: spawner.spawn(buttons::buttons_task(p.PE10, p.PE11).unwrap());

// use embassy_stm32::{
//     gpio::{Input, Pull},
//     peripherals, Peri,
// };
// use embassy_time::{Duration, Timer};
// use jzf407_logic::debouncer::{Debouncer, SAMPLE_PERIOD_MS};
//
// #[embassy_executor::task]
// pub async fn buttons_task(
//     s1_pin: Peri<'static, peripherals::PE10>,
//     s2_pin: Peri<'static, peripherals::PE11>,
// ) {
//     let s1 = Input::new(s1_pin, Pull::Up);
//     let s2 = Input::new(s2_pin, Pull::Up);
//
//     let mut deb_s1 = Debouncer::new(false);
//     let mut deb_s2 = Debouncer::new(false);
//
//     loop {
//         Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;
//
//         let _ = deb_s1.update(s1.is_low());
//         let _ = deb_s2.update(s2.is_low());
//     }
// }
