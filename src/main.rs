#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_net::{Ipv4Address, Ipv4Cidr, StackResources, StaticConfigV4};
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
    // 168 MHz from 25 MHz HSE crystal (HseMode::Oscillator confirmed working).
    // APB1 = 42 MHz, APB2 = 84 MHz, TIM2 input = 84 MHz.
    // tick-hz-1_000_000 → TIM2 PSC = 83 (fits in u16). Do NOT use tick-hz-1_000
    // at 168 MHz — that would require PSC=83999 which overflows u16 and panics.
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
    let p = embassy_stm32::init(p_cfg);

    info!("JZF407VET6 booting — 168 MHz HSE PLL OK");
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

    // ---- Ethernet RMII + DP83848 ----
    // PA1=REF_CLK, PA2=MDIO, PA7=CRS_DV
    // PB11=TX_EN, PB12=TXD0, PB13=TXD1
    // PC1=MDC,  PC4=RXD0, PC5=RXD1
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
        queue, p.ETH, Irqs,
        p.PA1,  // REF_CLK
        p.PA7,  // CRS_DV
        p.PC4,  // RXD0
        p.PC5,  // RXD1
        p.PB12, // TXD0
        p.PB13, // TXD1
        p.PB11, // TX_EN
        mac_addr, p.ETH_SMA, p.PA2, // MDIO
        p.PC1,  // MDC
    );

    let [ga, gb, gc, gd] = net_cfg.gateway;
    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(a, b, c, d), net_cfg.prefix_len),
        gateway: Some(Ipv4Address::new(ga, gb, gc, gd)),
        dns_servers: heapless::Vec::new(),
    });

    let resources = STACK_RESOURCES.init(StackResources::<4>::new());
    let seed = {
        let u = raw_uid;
        u64::from_le_bytes([u[0], u[1], u[2], u[3], u[4], u[5], u[6], u[7]])
    };
    let (stack, runner) = embassy_net::new(eth, net_config, resources, seed);

    spawner.spawn(net::net_task(runner).unwrap());
    spawner.spawn(mqtt::mqtt_task(stack.clone(), net_cfg.clone(), reset_reason).unwrap());
    spawner.spawn(web::web_task(stack, net_cfg).unwrap());
}
