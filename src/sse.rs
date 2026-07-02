//! Server-Sent Events push for the dashboard: state sampled every `POLL_MS`,
//! pushed only on diff. Auth and routing live in `web`.

use core::fmt::Write as _;
use core::sync::atomic::Ordering;
use defmt::info;
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::Write;
use heapless::String as HString;

// No Content-Length + keep-alive holds the response open; X-Accel-Buffering
// and no-cache defeat intermediary buffering.
const SSE_HEAD: &str = "HTTP/1.1 200 OK\r\n\
Content-Type: text/event-stream\r\n\
Cache-Control: no-cache\r\n\
Connection: keep-alive\r\n\
X-Accel-Buffering: no\r\n\r\n";

const POLL_MS: u64 = 150;

// The ':' comment line is ignored by EventSource but keeps NAT/proxies from
// reaping the connection and makes a dead peer fail the write promptly.
const KEEPALIVE_SECS: u64 = 15;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Snapshot {
    relay: bool,
    led1: bool,
    led2: bool,
    link: bool,
    mqtt: bool,
}

impl Snapshot {
    fn sample(stack: Stack<'static>) -> Self {
        Self {
            relay: crate::OUTPUTS.get_relay(),
            led1: crate::OUTPUTS.get_led(crate::LedId::Led1),
            led2: crate::OUTPUTS.get_led(crate::LedId::Led2),
            link: stack.is_link_up(),
            mqtt: crate::mqtt::MQTT_ONLINE.load(Ordering::Relaxed),
        }
    }

    fn write_event<const N: usize>(&self, buf: &mut HString<N>) {
        let _ = write!(
            buf,
            "data: {{\"relay\":{},\"led1\":{},\"led2\":{},\"link\":{},\"mqtt\":{},\"up\":{}}}\n\n",
            self.relay as u8,
            self.led1 as u8,
            self.led2 as u8,
            self.link as u8,
            self.mqtt as u8,
            Instant::now().as_secs(),
        );
    }
}

pub async fn serve_events(socket: &mut TcpSocket<'_>, stack: Stack<'static>) {
    info!("SSE: client connected");

    // A long-lived stream must not inherit the short request timeout; a dead
    // peer is detected by a failing keep-alive write instead.
    socket.set_timeout(None);

    if socket.write_all(SSE_HEAD.as_bytes()).await.is_err() {
        return;
    }

    let mut last = Snapshot::sample(stack);
    let mut ev: HString<128> = HString::new();
    last.write_event(&mut ev);
    if socket.write_all(ev.as_bytes()).await.is_err() {
        return;
    }
    let _ = socket.flush().await;

    let mut next_keepalive = Instant::now() + Duration::from_secs(KEEPALIVE_SECS);
    loop {
        Timer::after(Duration::from_millis(POLL_MS)).await;

        let now = Snapshot::sample(stack);
        if now != last {
            last = now;
            ev.clear();
            now.write_event(&mut ev);
            if socket.write_all(ev.as_bytes()).await.is_err() {
                return;
            }
            let _ = socket.flush().await;
            next_keepalive = Instant::now() + Duration::from_secs(KEEPALIVE_SECS);
        } else if Instant::now() >= next_keepalive {
            if socket.write_all(b": keepalive\n\n").await.is_err() {
                return;
            }
            let _ = socket.flush().await;
            next_keepalive = Instant::now() + Duration::from_secs(KEEPALIVE_SECS);
        }
    }
}
