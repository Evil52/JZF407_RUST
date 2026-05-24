//! Server-Sent Events (SSE) push for the dashboard.
//!
//! Why SSE and not WebSocket: SSE is just a long-lived HTTP/1.1 response with
//! `Content-Type: text/event-stream`. No upgrade handshake, no frame parser —
//! the server writes `data: {json}\n\n` whenever state changes and the browser
//! delivers it to `EventSource.onmessage`. WebSocket would need a SHA-1/base64
//! handshake plus frame (de)framing for no benefit here: the page only consumes
//! pushes (control still goes through MQTT and POST forms).
//!
//! This module is just the stream body: `serve_events` takes an already-accepted,
//! already-authenticated socket whose request was `GET /events`, and holds it
//! open pushing state. Routing/auth live in `web::web_task`, which runs as a
//! 2-socket pool so either socket can serve `/events` or a normal request — no
//! request ever lands on a socket that can't handle it.
//!
//! State source of truth = the physical pins / atomics, exactly like `/state`.
//! Rather than have every writer (relay_task, mqtt handle_event, link state…)
//! fire a signal, this POLLS the snapshot every `POLL_MS` and pushes only when it
//! differs from the last one sent. That catches *all* changes — including link
//! up/down, which smoltcp flips internally with no writer to hook — and keeps
//! in-air traffic to "on change only". Polling pin/atomic reads is nanoseconds;
//! nothing goes out the wire while state is steady.

use core::fmt::Write as _;
use core::sync::atomic::Ordering;
use defmt::info;
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::Write;
use heapless::String as HString;

/// SSE response head. No `Content-Length` + `Connection: keep-alive` = the
/// browser holds the response open and streams events. `X-Accel-Buffering: no`
/// and `no-cache` defeat any intermediary buffering so events arrive promptly.
const SSE_HEAD: &str = "HTTP/1.1 200 OK\r\n\
Content-Type: text/event-stream\r\n\
Cache-Control: no-cache\r\n\
Connection: keep-alive\r\n\
X-Accel-Buffering: no\r\n\r\n";

/// How often the stream samples state for a diff. 150 ms → a change reaches the
/// browser in well under a fifth of a second (reads as instant), while idle
/// state sends nothing.
const POLL_MS: u64 = 150;

/// Comment-line keep-alive cadence. A line starting with ':' is ignored by the
/// EventSource parser but still moves bytes, so a `write` fails promptly once the
/// client has gone away (freeing the socket) and NAT/proxies don't reap an
/// idle-looking connection.
const KEEPALIVE_SECS: u64 = 15;

/// One bit-packed snapshot of everything the dashboard shows live. Cheap to
/// build and compare, so the diff that gates a push is a single `!=`.
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

    /// Format as one SSE event: `data: {json}\n\n`. `up` (uptime secs) rides along
    /// so the client can re-sync its locally-ticking uptime on each push.
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

/// Stream live state over an accepted, authenticated `GET /events` socket until
/// the client goes away. Returns when any write fails (peer gone) so the caller
/// can close the socket and accept the next connection. The caller owns the
/// socket lifecycle (accept/close); we only stream.
pub async fn serve_events(socket: &mut TcpSocket<'_>, stack: Stack<'static>) {
    info!("SSE: client connected");

    // A long-lived stream must not inherit the short request timeout: drop it so
    // an idle (but alive) stream isn't torn down. A dead peer is detected by a
    // failing keep-alive write instead. Re-armed by the caller for the next req.
    socket.set_timeout(None);

    if socket.write_all(SSE_HEAD.as_bytes()).await.is_err() {
        return;
    }

    // Paint the truth immediately so a fresh page doesn't wait for the next change.
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
