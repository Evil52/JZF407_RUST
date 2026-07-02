use embassy_stm32::{peripherals, wdg::IndependentWatchdog, Peri};
use embassy_time::{Duration, Timer};

#[embassy_executor::task]
pub async fn watchdog_task(iwdg: Peri<'static, peripherals::IWDG>) {
    let mut wdg = IndependentWatchdog::new(iwdg, 20_000_000); // 20 s in µs
    wdg.unleash();
    loop {
        wdg.pet();
        Timer::after(Duration::from_millis(1_500)).await;
    }
}
