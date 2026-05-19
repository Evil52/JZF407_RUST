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
    types::{MqttBinary, MqttString, QoS, TopicFilter, TopicName},
    Bytes,
};

/// Signal from buttons_task → mqtt_task: relay state changed.
pub static RELAY_CHANGE: Signal<CriticalSectionRawMutex, bool> = Signal::new();

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
    stack.wait_config_up().await;
    info!("MQTT: network up");

    let mut rx_buf = [0u8; 1536];
    let mut tx_buf = [0u8; 1536];
    let mut mqtt_buf = [0u8; 2048];

    let [ba, bb, bc, bd] = cfg.broker_ip;
    let broker_addr = embassy_net::IpEndpoint::new(
        embassy_net::IpAddress::Ipv4(embassy_net::Ipv4Address::new(ba, bb, bc, bd)),
        cfg.broker_port,
    );

    // Grace period: ignore retained messages for 3 s after connect
    const GRACE_MS: u64 = 3_000;
    // Heartbeat every 10 s
    const HB_INTERVAL_MS: u64 = 10_000;

    let diag_str = reset_reason.as_str();

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
        socket.set_timeout(Some(Duration::from_secs(30)));

        info!("MQTT: connecting to broker {}:{}", ba, cfg.broker_port);
        if socket.connect(broker_addr).await.is_err() {
            warn!("MQTT: TCP connect failed, retry in 1 s");
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        let mut bump = BumpBuffer::new(&mut mqtt_buf);

        // Client<'_, N, B, MAX_SUBSCRIBES, RECEIVE_MAX, SEND_MAX, MAX_SUB_IDS>
        let mut client = Client::<'_, _, _, 8, 4, 4, 1>::new(&mut bump);

        let client_id_str = core::str::from_utf8(cfg.client_id.as_ref())
            .unwrap_or("stm32-jzf407")
            .trim_end_matches('\0');
        let client_id = MqttString::from_str(client_id_str).ok();

        let will_topic = TopicName::new(MqttString::from_str(MQTT_TOPIC_STATUS).unwrap()).unwrap();
        let will_msg = MqttBinary::try_from(b"offline" as &[u8]).unwrap();
        let will_opts = WillOptions::new(will_topic, will_msg).retain();

        // keep_alive = 120 s (broker timeout); heartbeat publish every 10 s covers unreliable networks
        let connect_opts = ConnectOptions::new()
            .clean_start()
            .keep_alive(KeepAlive::Seconds(core::num::NonZero::new(120).unwrap()))
            .will(will_opts);

        match client.connect(socket, &connect_opts, client_id).await {
            Ok(_) => info!("MQTT: connected"),
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

        // Subscribe to all control topics (fire-and-forget; suback arrives later in poll loop)
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

        // Main event loop
        loop {
            // Check if time for heartbeat
            let now = embassy_time::Instant::now();
            if now >= next_heartbeat {
                let _ = publish_qos0(&mut client, MQTT_TOPIC_HEARTBEAT, b"1").await;
                next_heartbeat = now + Duration::from_millis(HB_INTERVAL_MS);
            }

            // Calculate timeout until next scheduled action
            let hb_left = next_heartbeat.saturating_duration_since(embassy_time::Instant::now());
            let timeout = hb_left.min(Duration::from_secs(1));

            // Poll for incoming MQTT packet OR relay change signal OR timeout
            let result = embassy_futures::select::select3(
                client.poll(),
                RELAY_CHANGE.wait(),
                Timer::after(timeout),
            )
            .await;

            match result {
                embassy_futures::select::Either3::First(Ok(event)) => {
                    if embassy_time::Instant::now() < grace_deadline {
                        // still in grace period — ignore retained messages
                    } else {
                        // Process event; if Ping, publish pong using &mut client
                        handle_event(&mut client, event).await;
                    }
                }
                embassy_futures::select::Either3::First(Err(e)) => {
                    error!("MQTT: poll error {:?}", e);
                    break;
                }
                embassy_futures::select::Either3::Second(relay_on) => {
                    // Relay state change from button — publish to stm32/relay
                    let payload = if relay_on { b"1" } else { b"0" };
                    if publish_qos0(&mut client, MQTT_TOPIC_RELAY, payload)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                embassy_futures::select::Either3::Third(_) => {
                    // timeout — ping keepalive
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
            OutputCmd::Led1(on) => crate::OUTPUTS.set(LedId::Led1, on),
            OutputCmd::Led2(on) => crate::OUTPUTS.set(LedId::Led2, on),
            OutputCmd::AllLeds(on) => {
                crate::OUTPUTS.set(LedId::Led1, on);
                crate::OUTPUTS.set(LedId::Led2, on);
            }
            OutputCmd::Relay(on) => RELAY_CHANGE.signal(on),
            OutputCmd::Ping => {
                // Reply with pong for RTT measurement
                let _ = publish_qos0(client, MQTT_TOPIC_PONG, b"1").await;
            }
            OutputCmd::Reboot => {
                crate::fault_marker::mark_remote_reboot();
                cortex_m::peripheral::SCB::sys_reset();
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
