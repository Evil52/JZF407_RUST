#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_net::{Ipv4Address, Ipv4Cidr, StackResources, StaticConfigV4};
use embassy_stm32::{
    bind_interrupts,
    eth::{self, PacketQueue},
    gpio::{Level, Output, OutputOpenDrain, Speed},
    i2c, peripherals,
    rcc::{
        AHBPrescaler, APBPrescaler, Hse, HseMode, Pll, PllMul, PllPDiv, PllPreDiv, PllQDiv,
        PllSource, Sysclk,
    },
    time::Hertz,
    uid, Config,
};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_persist as _};

mod buttons;
mod config;
mod eeprom;
mod fault_marker;
mod heartbeat;
mod mqtt;
mod net;
mod outputs;
mod persistence;
mod sse;
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
static STACK_RESOURCES: StaticCell<StackResources<8>> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // 168 MHz from 25 MHz HSE. Do NOT use tick-hz-1_000 at 168 MHz — TIM2 would
    // need PSC=83999, which overflows u16 and panics inside embassy_stm32::init().
    let mut rcc_cfg = embassy_stm32::rcc::Config::default();
    rcc_cfg.hse = Some(Hse {
        freq: Hertz(25_000_000),
        mode: HseMode::Oscillator,
    });
    rcc_cfg.pll_src = PllSource::HSE;
    rcc_cfg.pll = Some(Pll {
        prediv: PllPreDiv::DIV25,
        mul: PllMul::MUL336,
        divp: Some(PllPDiv::DIV2), // SYSCLK = 168 MHz
        divq: Some(PllQDiv::DIV7), // 48 MHz (USB)
        divr: None,
    });
    rcc_cfg.ahb_pre = AHBPrescaler::DIV1;
    rcc_cfg.apb1_pre = APBPrescaler::DIV4; // APB1 = 42 MHz, TIM2 = 84 MHz
    rcc_cfg.apb2_pre = APBPrescaler::DIV2; // APB2 = 84 MHz
    rcc_cfg.sys = Sysclk::PLL1_P;

    let mut p_cfg = Config::default();
    p_cfg.rcc = rcc_cfg;
    let mut p = embassy_stm32::init(p_cfg);

    // DEMCR.TRCENA: without CoreSight trace enabled, ETH DMA does not run
    // correctly on STM32F4 with no debugger attached.
    //
    // Safety: DEMCR (0xE000EDFC) is an architecturally-fixed, always-mapped,
    // word-aligned Cortex-M register; no Rust object lives there. The RMW is
    // race-free — early boot, before tasks are spawned or interrupts enabled.
    unsafe {
        const DEMCR: *mut u32 = 0xE000_EDFC as *mut u32;
        core::ptr::write_volatile(DEMCR, core::ptr::read_volatile(DEMCR) | (1 << 24));
    }

    // Keep ETH clocked in WFI sleep: the executor sleeps the core when idle and
    // without these LP-enable bits ETH RX dies with no debugger attached.
    {
        use embassy_stm32::pac::RCC;
        RCC.ahb1lpenr().modify(|w| {
            w.set_ethlpen(true);
            w.set_ethrxlpen(true);
            w.set_ethtxlpen(true);
        });
    }

    info!("JZF407VET6 booting — 168 MHz HSE PLL OK");
    embassy_time::Timer::after(embassy_time::Duration::from_millis(100)).await;

    let reset_reason = fault_marker::read_and_clear();
    info!("Reset: {}", reset_reason.as_str());

    if let Some(msg) = panic_persist::get_panic_message_utf8() {
        defmt::error!("PANIC from last run: {}", msg);
    }

    let led3 = Output::new(p.PE15, Level::High, Speed::Low);
    spawner.spawn(heartbeat::heartbeat_led_task(led3).unwrap());

    spawner.spawn(watchdog::watchdog_task(p.IWDG).unwrap());

    // I2C1 bus recovery: after sys_reset the AT24C02 may be mid-transaction
    // holding SDA low; the first blocking read would then hang forever.
    // 9 SCL pulses + STOP unstick it before the I2C peripheral is initialised.
    {
        const HALF: u32 = 840; // ~5 µs half-period for 100 kHz at 168 MHz
        let mut scl = OutputOpenDrain::new(p.PB8.reborrow(), Level::High, Speed::Low);
        let mut sda = OutputOpenDrain::new(p.PB9.reborrow(), Level::High, Speed::Low);
        for _ in 0..9 {
            scl.set_low();
            cortex_m::asm::delay(HALF);
            scl.set_high();
            cortex_m::asm::delay(HALF);
            if sda.is_high() {
                break;
            }
        }
        scl.set_low();
        cortex_m::asm::delay(HALF);
        sda.set_low();
        cortex_m::asm::delay(HALF);
        scl.set_high();
        cortex_m::asm::delay(HALF);
        sda.set_high();
        cortex_m::asm::delay(HALF);
        drop(scl);
        drop(sda);
    }
    info!("I2C recovery done");

    let mut i2c_cfg = i2c::Config::default();
    i2c_cfg.frequency = Hertz::khz(100);
    let i2c = i2c::I2c::new_blocking(p.I2C1, p.PB8, p.PB9, i2c_cfg);
    eeprom::Eeprom::new(i2c).init_global();

    info!("loading config...");
    let net_cfg = config::load_config();
    info!("config loaded");
    let saved_state = persistence::load_state();
    info!("state loaded");
    let [a, b, c, d] = net_cfg.ip;
    info!("IP: {}.{}.{}.{}/{}", a, b, c, d, net_cfg.prefix_len);

    OUTPUTS.register(outputs::OutputPins {
        led1: Output::new(p.PE13, Level::High, Speed::Low),
        led2: Output::new(p.PE14, Level::High, Speed::Low),
        relay: Output::new(p.PD4, Level::Low, Speed::Low), // active-HIGH: Low = off at boot
    });

    OUTPUTS.set(LedId::Led1, saved_state.led1);
    OUTPUTS.set(LedId::Led2, saved_state.led2);

    spawner.spawn(outputs::relay_task(&OUTPUTS).unwrap());

    // buttons_task deliberately not spawned — see src/buttons.rs.

    let raw_uid = uid::uid();
    let mac_addr = [
        0x02, raw_uid[0], raw_uid[1], raw_uid[2], raw_uid[3], raw_uid[4],
    ];
    info!(
        "MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac_addr[0], mac_addr[1], mac_addr[2], mac_addr[3], mac_addr[4], mac_addr[5]
    );

    let queue = PACKET_QUEUE.init(PacketQueue::<4, 4>::new());

    let eth = eth::Ethernet::new(
        queue, p.ETH, Irqs, p.PA1,  // REF_CLK
        p.PA7,  // CRS_DV
        p.PC4,  // RXD0
        p.PC5,  // RXD1
        p.PB12, // TXD0
        p.PB13, // TXD1
        p.PB11, // TX_EN
        mac_addr, p.ETH_SMA, p.PA2, // MDIO
        p.PC1, // MDC
    );

    let [ga, gb, gc, gd] = net_cfg.gateway;
    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(a, b, c, d), net_cfg.prefix_len),
        gateway: Some(Ipv4Address::new(ga, gb, gc, gd)),
        dns_servers: heapless::Vec::new(),
    });

    let resources = STACK_RESOURCES.init(StackResources::<8>::new());
    let seed = {
        let u = raw_uid;
        u64::from_le_bytes([u[0], u[1], u[2], u[3], u[4], u[5], u[6], u[7]])
    };
    let (stack, runner) = embassy_net::new(eth, net_config, resources, seed);

    // DP83848 needs up to 3 s for auto-negotiation after soft reset.
    embassy_time::Timer::after(embassy_time::Duration::from_millis(3000)).await;

    spawner.spawn(net::net_task(runner).unwrap());
    spawner.spawn(mqtt::mqtt_task(stack, net_cfg.clone(), reset_reason).unwrap());
    spawner.spawn(web::web_task(stack, net_cfg.clone(), reset_reason).unwrap());
    spawner.spawn(web::web_task_b(stack, net_cfg, reset_reason).unwrap());
}
