use core::sync::atomic::{AtomicBool, Ordering};
use crate::fault_marker::ResetReason;
use crate::outputs::LedId;
use defmt::{error, info, warn};
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use jzf407_logic::config::NetworkConfig;
use rust_mqtt::{
    buffer::BumpBuffer,
    client::{
        event::Event,
        options::{
            ConnectOptions, PublicationOptions, SubscriptionOptions, TopicReference, WillOptions,
        },
        Client,
    },
    config::KeepAlive,
    types::{MqttBinary, MqttString, TopicFilter, TopicName},
    Bytes,
};

/// Relay-changed notification from buttons_task / web_task → mqtt_task, so the
/// new state gets published to `stm32/relay`. Deliberately NOT signalled from
/// the MQTT receive path (handle_event): the broker echoes our own publish back
/// (we subscribe to that topic), which would loop forever. See handle_event.
pub static RELAY_CHANGE: Signal<CriticalSectionRawMutex, bool> = Signal::new();

/// True only while the MQTT client is connected to the broker. Set by mqtt_task,
/// read by web_task for the live "MQTT" status indicator. Plain atomic — it is a
/// single bool with no ordering requirements against other state.
pub static MQTT_ONLINE: AtomicBool = AtomicBool::new(false);

const MQTT_TOPIC_LED1: &str = "stm32/led/1";
const MQTT_TOPIC_LED2: &str = "stm32/led/2";
const MQTT_TOPIC_LED_ALL: &str = "stm32/led/all";
const MQTT_TOPIC_RELAY: &str = "stm32/relay";
const MQTT_TOPIC_PING: &str = "stm32/ping";
const MQTT_TOPIC_REBOOT: &str = "stm32/cmd/reboot";
const MQTT_TOPIC_STATUS: &str = "stm32/status";
const MQTT_TOPIC_DIAG: &str = "stm32/diag";
const MQTT_TOPIC_HEARTBEAT: &str = "stm32/heartbeat";
const MQTT_TOPIC_PONG: &str = "stm32/pong";

const SUBSCRIBE_TOPICS: &[&str] = &[
    MQTT_TOPIC_LED1,
    MQTT_TOPIC_LED2,
    MQTT_TOPIC_LED_ALL,
    MQTT_TOPIC_RELAY,
    MQTT_TOPIC_PING,
    MQTT_TOPIC_REBOOT,
];

#[embassy_executor::task]
pub async fn mqtt_task(stack: Stack<'static>, cfg: NetworkConfig, reset_reason: ResetReason) {
    info!("MQTT: waiting for link...");
    stack.wait_link_up().await;
    info!("MQTT: link up, waiting for config...");
    stack.wait_config_up().await;
    info!("MQTT: network up");

    static RX_BUF: static_cell::StaticCell<[u8; 1536]> = static_cell::StaticCell::new();
    static TX_BUF: static_cell::StaticCell<[u8; 1536]> = static_cell::StaticCell::new();
    static MQTT_BUF: static_cell::StaticCell<[u8; 2048]> = static_cell::StaticCell::new();
    let rx_buf = RX_BUF.init([0u8; 1536]);
    let tx_buf = TX_BUF.init([0u8; 1536]);
    let mqtt_buf: &'static mut [u8; 2048] = MQTT_BUF.init([0u8; 2048]);

    let [ba, bb, bc, bd] = cfg.broker_ip;
    let broker_addr = embassy_net::IpEndpoint::new(
        embassy_net::IpAddress::Ipv4(embassy_net::Ipv4Address::new(ba, bb, bc, bd)),
        cfg.broker_port,
    );

    const GRACE_MS: u64 = 3_000;
    const HB_INTERVAL_MS: u64 = 10_000;

    let diag_str = reset_reason.as_str();

    loop {
        MQTT_ONLINE.store(false, Ordering::Relaxed);
        let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);
        socket.set_timeout(Some(Duration::from_secs(30)));

        info!(
            "MQTT: connecting to broker {}.{}.{}.{}:{}",
            ba, bb, bc, bd, cfg.broker_port
        );
        if socket.connect(broker_addr).await.is_err() {
            warn!("MQTT: TCP connect failed, retry in 1 s");
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        mqtt_buf.fill(0);
        let mut bump = BumpBuffer::new(mqtt_buf);

        let mut client = Client::<'_, _, _, 8, 4, 4, 1>::new(&mut bump);

        let client_id_str = core::str::from_utf8(cfg.client_id.as_ref())
            .unwrap_or("stm32-jzf407")
            .trim_end_matches('\0');
        let client_id = MqttString::from_str(client_id_str).ok();

        let will_topic = TopicName::new(MqttString::from_str(MQTT_TOPIC_STATUS).unwrap()).unwrap();
        let will_msg = MqttBinary::try_from(b"offline" as &[u8]).unwrap();
        let will_opts = WillOptions::new(will_topic, will_msg).retain();

        // keep_alive = 120 s (broker timeout); heartbeat publish every 10 s covers unreliable networks
        let mut connect_opts = ConnectOptions::new()
            .clean_start()
            .keep_alive(KeepAlive::Seconds(core::num::NonZero::new(120).unwrap()))
            .will(will_opts);

        // NOTE: credentials are sent in the CLEAR on port 1883 — see README.
        if !cfg.mqtt_user.is_empty() {
            if let Ok(u) = MqttString::from_str(cfg.mqtt_user.as_str()) {
                connect_opts = connect_opts.user_name(u);
            }
        }
        if !cfg.mqtt_pass.is_empty() {
            if let Ok(p) = MqttBinary::try_from(cfg.mqtt_pass.as_bytes()) {
                connect_opts = connect_opts.password(p);
            }
        }

        match client.connect(socket, &connect_opts, client_id).await {
            Ok(_) => {
                info!("MQTT: connected");
                MQTT_ONLINE.store(true, Ordering::Relaxed);
            }
            Err(e) => {
                error!("MQTT: connect error {:?}, retry in 1 s", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        }

        // Publish status=online (retained)
        let _ = publish_retained(&mut client, MQTT_TOPIC_STATUS, b"online").await;
        // Publish diag (retained)
        let _ = publish_retained(&mut client, MQTT_TOPIC_DIAG, diag_str.as_bytes()).await;

        // fire-and-forget; suback arrives in the poll loop below
        let sub_opts = SubscriptionOptions::new();
        let mut sub_ok = true;
        for &topic in SUBSCRIBE_TOPICS {
            let tf = match TopicFilter::new(MqttString::from_str(topic).unwrap()) {
                Some(tf) => tf,
                None => continue,
            };
            if client.subscribe(tf, sub_opts).await.is_err() {
                sub_ok = false;
                break;
            }
            // Do NOT wait for Suback here — poll below will process everything
        }
        if !sub_ok {
            warn!("MQTT: subscribe request failed");
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        info!("MQTT: subscribed to all topics");

        let grace_deadline = embassy_time::Instant::now() + Duration::from_millis(GRACE_MS);
        let mut next_heartbeat =
            embassy_time::Instant::now() + Duration::from_millis(HB_INTERVAL_MS);

        loop {
            let now = embassy_time::Instant::now();
            if now >= next_heartbeat {
                let _ = publish_qos0(&mut client, MQTT_TOPIC_HEARTBEAT, b"1").await;
                next_heartbeat = now + Duration::from_millis(HB_INTERVAL_MS);
            }

            let hb_left = next_heartbeat.saturating_duration_since(embassy_time::Instant::now());
            let timeout = hb_left.min(Duration::from_secs(1));

            let result = embassy_futures::select::select3(
                client.poll(),
                RELAY_CHANGE.wait(),
                Timer::after(timeout),
            )
            .await;

            match result {
                embassy_futures::select::Either3::First(Ok(event)) => {
                    if embassy_time::Instant::now() >= grace_deadline {
                        handle_event(&mut client, event).await;
                    }
                }
                embassy_futures::select::Either3::First(Err(e)) => {
                    error!("MQTT: poll error {:?}", e);
                    break;
                }
                embassy_futures::select::Either3::Second(relay_on) => {
                    let payload = if relay_on { b"1" } else { b"0" };
                    if publish_qos0(&mut client, MQTT_TOPIC_RELAY, payload)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                embassy_futures::select::Either3::Third(_) => {
                    if client.ping().await.is_err() {
                        warn!("MQTT: ping failed");
                        break;
                    }
                }
            }
        }

        warn!("MQTT: disconnected, reconnecting in 1 s");
        Timer::after(Duration::from_secs(1)).await;
    }
}

/// Process an incoming MQTT event. Needs &mut client for publishing pong responses.
async fn handle_event<'c, N, B>(client: &mut Client<'c, N, B, 8, 4, 4, 1>, event: Event<'_, 1>)
where
    N: rust_mqtt::io::Transport,
    B: rust_mqtt::buffer::BufferProvider<'c>,
{
    let Event::Publish(pub_event) = event else {
        return;
    };
    let topic = pub_event.topic.as_ref().as_str();
    let payload = pub_event.message.as_bytes();

    use jzf407_logic::led_dispatch::{dispatch, dispatch_special, OutputCmd};

    if let Some(cmd) = dispatch(topic, payload).or_else(|| {
        // For ping/reboot, payload is irrelevant
        dispatch_special(topic)
    }) {
        match cmd {
            OutputCmd::Led1(on) => {
                crate::OUTPUTS.set(LedId::Led1, on);
                crate::persistence::save_led1(on).await;
            }
            OutputCmd::Led2(on) => {
                crate::OUTPUTS.set(LedId::Led2, on);
                crate::persistence::save_led2(on).await;
            }
            OutputCmd::AllLeds(on) => {
                crate::OUTPUTS.set(LedId::Led1, on);
                crate::OUTPUTS.set(LedId::Led2, on);
                crate::persistence::save_leds(on, on).await;
            }
            OutputCmd::Relay(on) => {
                // Relay is a momentary pulse: any truthy command fires a 2 s
                // pulse, falsy cancels it. Timing is owned by relay_task. Do NOT
                // signal RELAY_CHANGE here: that triggers a publish to
                // stm32/relay, which the broker would echo back (we subscribe to
                // it) and loop forever.
                if on { crate::outputs::pulse_relay() } else { crate::outputs::relay_off() }
            }
            OutputCmd::Ping => {
                // Reply with pong for RTT measurement
                let _ = publish_qos0(client, MQTT_TOPIC_PONG, b"1").await;
            }
            OutputCmd::Reboot => {
                defmt::warn!("MQTT: reboot command received, resetting");
                crate::fault_marker::mark_remote_reboot();
                crate::fault_marker::safe_reboot();
            }
            OutputCmd::Unknown => {}
        }
    }
}

async fn publish_qos0<'c, N, B>(
    client: &mut Client<'c, N, B, 8, 4, 4, 1>,
    topic: &str,
    payload: &[u8],
) -> Result<(), ()>
where
    N: rust_mqtt::io::Transport,
    B: rust_mqtt::buffer::BufferProvider<'c>,
{
    let topic_ref = TopicName::new(MqttString::from_str(topic).unwrap()).unwrap();
    let pub_opts = PublicationOptions::new(TopicReference::Name(topic_ref));
    client
        .publish(&pub_opts, Bytes::from(payload))
        .await
        .map(|_| ())
        .map_err(|_| ())
}

async fn publish_retained<'c, N, B>(
    client: &mut Client<'c, N, B, 8, 4, 4, 1>,
    topic: &str,
    payload: &[u8],
) -> Result<(), ()>
where
    N: rust_mqtt::io::Transport,
    B: rust_mqtt::buffer::BufferProvider<'c>,
{
    let topic_ref = TopicName::new(MqttString::from_str(topic).unwrap()).unwrap();
    let pub_opts = PublicationOptions::new(TopicReference::Name(topic_ref)).retain();
    client
        .publish(&pub_opts, Bytes::from(payload))
        .await
        .map(|_| ())
        .map_err(|_| ())
}
