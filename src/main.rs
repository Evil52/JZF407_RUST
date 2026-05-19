#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    eth::{self, PacketQueue},
    gpio::{Level, Output, Speed},
    i2c, peripherals,
    rcc::{
        AHBPrescaler, APBPrescaler, Hse, HseMode, Pll, PllMul, PllPDiv, PllPreDiv, PllQDiv,
        PllSource, Sysclk,
    },
    time::Hertz,
    uid, Config,
};
use static_cell::StaticCell;
use embassy_net::{Ipv4Address, Ipv4Cidr, StackResources, StaticConfigV4};
use {defmt_rtt as _, panic_persist as _};

#[cortex_m_rt::exception]
unsafe fn HardFault(_ef: &cortex_m_rt::ExceptionFrame) -> ! {
    // Blink PE15 (LED3) rapidly to indicate hard fault — visible without RTT
    use embassy_stm32::pac::{GPIOE, RCC};
    RCC.ahb1enr().modify(|w| w.set_gpioeen(true));
    let _ = RCC.ahb1enr().read();
    // Set PE15 as output (MODER bits 31:30 = 01)
    GPIOE.moder().modify(|w| w.set_moder(15, embassy_stm32::pac::gpio::vals::Moder::OUTPUT));
    loop {
        GPIOE.bsrr().write(|w| w.set_bs(15, true));  // set high
        for _ in 0..500_000u32 { cortex_m::asm::nop(); }
        GPIOE.bsrr().write(|w| w.set_br(15, true));  // set low
        for _ in 0..500_000u32 { cortex_m::asm::nop(); }
    }
}

mod buttons;
mod config;
mod eeprom;
mod fault_marker;
mod heartbeat;
mod mqtt;
mod net;
mod outputs;
mod persistence;
mod watchdog;
mod web;

bind_interrupts!(struct Irqs {
    I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
   I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
    ETH     => eth::InterruptHandler;
});

pub use outputs::{LedId, SharedOutputs};
pub static OUTPUTS: SharedOutputs = SharedOutputs::new();

static PACKET_QUEUE: StaticCell<PacketQueue<4, 4>> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Start on HSI (default) first so RTT is working before we touch PLL
    let p = embassy_stm32::init(Config::default());
    info!("HSI boot OK — now configuring PLL step by step");

    // Manual PLL configuration via PAC to identify exact crash point
    {
        use embassy_stm32::pac::{FLASH, PWR, RCC};

        info!("Step 1: enable PWR clock");
        RCC.apb1enr().modify(|w| w.set_pwren(true));
        let _ = RCC.apb1enr().read();

        info!("Step 2: set VOS=Scale1");
        PWR.cr1().modify(|w| w.set_vos(embassy_stm32::pac::pwr::vals::Vos::SCALE1));

        info!("Step 3: set flash wait states to 5 (WS5)");
        FLASH.acr().modify(|w| {
            // LATENCY bits [3:0] = 5 (5 wait states for 168 MHz at 3.3V)
            // Also enable prefetch, icache, dcache
            w.0 = (w.0 & !0x0F) | 5;
            // set PRFTEN (bit8), ICEN (bit9), DCEN (bit10)
            w.0 |= (1 << 8) | (1 << 9) | (1 << 10);
        });
        // Wait for flash latency to be applied
        while (FLASH.acr().read().0 & 0x0F) != 5 {}
        info!("Step 3 done: FLASH_ACR = {:08x}", FLASH.acr().read().0);

        info!("Step 4: configure PLL (HSI source, M=16, N=336, P=2, Q=7)");
        // PLLCFGR: PLLSRC=HSI(0), PLLM=16, PLLN=336, PLLP=DIV2(0b00), PLLQ=7
        // PLLCFGR layout: [5:0]=PLLM, [14:6]=PLLN, [17:16]=PLLP, [22]=PLLSRC, [27:24]=PLLQ
        let pllcfgr_val: u32 = 16       // PLLM=16
            | (336 << 6)                // PLLN=336
            | (0b00 << 16)              // PLLP=DIV2
            | (0 << 22)                 // PLLSRC=HSI
            | (7 << 24);               // PLLQ=7
        RCC.pllcfgr().write(|w| w.0 = pllcfgr_val);
        info!("PLL configured: PLLCFGR = {:08x}", RCC.pllcfgr().read().0);

        info!("Step 5: enable PLL");
        RCC.cr().modify(|w| w.set_pllon(true));
        info!("Waiting for PLLRDY...");
        let mut count = 0u32;
        while !RCC.cr().read().pllrdy() {
            count += 1;
            if count > 1_000_000 {
                info!("PLL timeout!");
                break;
            }
        }
        if RCC.cr().read().pllrdy() {
            info!("Step 5 done: PLL ready after {} iterations", count);
        }

        info!("Step 6: switch SYSCLK to PLL");
        // SW bits [1:0] in RCC_CFGR: 00=HSI, 01=HSE, 10=PLL
        RCC.cfgr().modify(|w| {
            w.0 = (w.0 & !0x3) | 0x2; // SW=PLL
        });
        info!("Waiting for SWS=PLL...");
        let mut count = 0u32;
        while (RCC.cfgr().read().0 & (0x3 << 2)) != (0x2 << 2) {
            count += 1;
            if count > 1_000_000 { break; }
        }
        info!("Step 6 done: SYSCLK switched to PLL, count={}", count);

        // Set AHB/APB prescalers
        info!("Step 7: set bus prescalers (AHB/1, APB1/4, APB2/2)");
        RCC.cfgr().modify(|w| {
            w.0 = (w.0 & !0b0000_0000_1111_1111_0000_0000_0000_0000)
                | (0b0000 << 4)   // HPRE=DIV1
                | (0b101 << 10)   // PPRE1=DIV4
                | (0b100 << 13);  // PPRE2=DIV2
        });
        info!("Step 7 done: CFGR = {:08x}", RCC.cfgr().read().0);
    }

    info!("PLL 168 MHz configured manually — SUCCESS");
    embassy_time::Timer::after(embassy_time::Duration::from_millis(100)).await;

    let reset_reason = fault_marker::read_and_clear();
    info!("Reset: {}", reset_reason.as_str());

    // ---- Heartbeat ----
    let led3 = Output::new(p.PE15, Level::High, Speed::Low);
    spawner.spawn(heartbeat::heartbeat_led_task(led3).unwrap());

    // ---- Watchdog ----
    spawner.spawn(watchdog::watchdog_task(p.IWDG).unwrap());

    // ---- I2C1 blocking for AT24C02 ----
    let mut i2c_cfg = i2c::Config::default();
    i2c_cfg.frequency = Hertz::khz(100);
    let i2c = i2c::I2c::new_blocking(p.I2C1, p.PB8, p.PB9, i2c_cfg);
    eeprom::Eeprom::new(i2c).init_global();

    // ---- Load network config + persisted output state from EEPROM ----
    let net_cfg = config::load_config();
    let saved_state = persistence::load_state();
    let [a, b, c, d] = net_cfg.ip;
    info!("IP: {}.{}.{}.{}/{}", a, b, c, d, net_cfg.prefix_len);

    // ---- Buttons + relay ----
    let relay = Output::new(p.PD4, Level::High, Speed::Low);
    spawner.spawn(buttons::buttons_task(p.PE10, p.PE11, relay).unwrap());

    // ---- LED outputs ----
    OUTPUTS.register(outputs::OutputPins {
        led1: Output::new(p.PE13, Level::High, Speed::Low),
        led2: Output::new(p.PE14, Level::High, Speed::Low),
    });

    // Apply persisted state to LEDs
    OUTPUTS.set(LedId::Led1, saved_state.led1);
    OUTPUTS.set(LedId::Led2, saved_state.led2);

    info!("PLL test OK — looping");
    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(5)).await;
        info!("alive at 168MHz HSI");
    }
}
