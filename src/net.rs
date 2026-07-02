use embassy_net::Runner;
use embassy_stm32::eth::{Ethernet, GenericPhy, Sma};

#[embassy_executor::task]
pub async fn net_task(
    mut runner: Runner<
        'static,
        Ethernet<
            'static,
            embassy_stm32::peripherals::ETH,
            GenericPhy<Sma<'static, embassy_stm32::peripherals::ETH_SMA>>,
        >,
    >,
) -> ! {
    runner.run().await
}
