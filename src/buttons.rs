// ============================================================================
// buttons.rs — DISABLED (задача запущена, но действий не выполняет)
//
// Кнопки S1 (PE10) / S2 (PE11) физически читаются и дебаунсируются,
// но результат нигде не используется. Код закомментирован до реализации логики.
//
// ----------------------------------------------------------------------------
// КАК ПОДКЛЮЧИТЬ КНОПКУ S3
// ----------------------------------------------------------------------------
// 1. Добавь пин в сигнатуру tasks:
//      s3_pin: Peri<'static, peripherals::PExx>,  // выбери нужный пин
//
// 2. В main.rs передай пин:
//      spawner.spawn(buttons::buttons_task(p.PE10, p.PE11, p.PExx).unwrap());
//
// 3. Внутри задачи добавь:
//      let s3 = Input::new(s3_pin, Pull::Up);
//      let mut deb_s3 = Debouncer::new(false);
//    и в loop:
//      if let Some(Edge::Rising) = deb_s3.update(s3.is_low()) { ... }
//
// ----------------------------------------------------------------------------
// КАК ПОДКЛЮЧИТЬ УПРАВЛЕНИЕ ВЫХОДАМИ ПО НАЖАТИЮ
// ----------------------------------------------------------------------------
// Импортировать Edge из debouncer-крейта:
//      use jzf407_logic::debouncer::Edge;
//
// S1 → toggle LED1:
//      if let Some(Edge::Rising) = deb_s1.update(s1.is_low()) {
//          let new = !crate::OUTPUTS.get_led(crate::LedId::Led1);
//          crate::OUTPUTS.set(crate::LedId::Led1, new);
//          crate::persistence::save_led1(new).await; // сохранить в EEPROM
//          crate::mqtt::RELAY_CHANGE.signal(false);  // опционально: сообщить MQTT
//      }
//
// S2 → toggle LED2:
//      if let Some(Edge::Rising) = deb_s2.update(s2.is_low()) {
//          let new = !crate::OUTPUTS.get_led(crate::LedId::Led2);
//          crate::OUTPUTS.set(crate::LedId::Led2, new);
//          crate::persistence::save_led2(new).await;
//      }
//
// S3 → pulse relay:
//      if let Some(Edge::Rising) = deb_s3.update(s3.is_low()) {
//          crate::outputs::pulse_relay(); // 2s моностабильный импульс
//          crate::mqtt::RELAY_CHANGE.signal(true); // опубликовать stm32/relay
//      }
//
// ----------------------------------------------------------------------------
// КАК ПРОБРОСИТЬ НАЖАТИЕ КНОПКИ В MQTT
// ----------------------------------------------------------------------------
// RELAY_CHANGE (в mqtt.rs) — это Signal<_, bool>, который mqtt_task слушает
// и публикует в топик stm32/relay при получении сигнала.
//
// Для кнопок нужны отдельные сигналы под каждый выход:
//
//   pub static LED1_CHANGE: Signal<CriticalSectionRawMutex, bool> = Signal::new();
//   pub static LED2_CHANGE: Signal<CriticalSectionRawMutex, bool> = Signal::new();
//
// Добавить в mqtt_task (select3 → select5):
//   embassy_futures::select::select(
//       ...,
//       LED1_CHANGE.wait(),
//       LED2_CHANGE.wait(),
//   )
// И при получении публиковать в stm32/led/1 и stm32/led/2.
//
// Альтернатива — использовать единый канал с enum-командой:
//   pub enum ButtonCmd { Led1(bool), Led2(bool), Relay(bool) }
//   pub static BUTTON_CMD: Channel<CriticalSectionRawMutex, ButtonCmd, 4> = Channel::new();
// ============================================================================

// use embassy_stm32::{
//     gpio::{Input, Pull},
//     Peri,
//     peripherals,
// };
// use embassy_time::{Duration, Timer};
// use jzf407_logic::debouncer::{Debouncer, SAMPLE_PERIOD_MS};

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
