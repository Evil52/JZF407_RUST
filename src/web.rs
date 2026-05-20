use defmt::{info, warn};
use embassy_net::{tcp::TcpSocket, Stack};
use embedded_io_async::Write;
use embassy_time::{Duration, Timer};
use heapless::String as HString;
use jzf407_logic::config::{parse_ipv4, parse_port, NetworkConfig};

const HTTP_OK: &str =
    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n";
const HTTP_303: &str = "HTTP/1.1 303 See Other\r\nLocation: /\r\nConnection: close\r\n\r\n";
const HTTP_400: &str = "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nBad Request";
const HTTP_404: &str = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\nNot Found";

const PAGE_HEAD: &str = "<!DOCTYPE html><html lang='en'><head><meta charset='utf-8'>\
<meta name='viewport' content='width=device-width,initial-scale=1'><title>JZF407VET6</title>\
<style>\
*{box-sizing:border-box;margin:0;padding:0}\
body{font-family:system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;background:#eef2f6;color:#1e293b;line-height:1.5;padding:24px 16px}\
.wrap{max-width:460px;margin:0 auto}\
.card{background:#fff;border-radius:16px;box-shadow:0 4px 24px rgba(15,23,42,.08);margin-bottom:18px;overflow:hidden}\
.hd{background:linear-gradient(135deg,#4f46e5,#7c3aed);color:#fff;padding:22px 24px}\
.hd h1{font-size:19px;font-weight:600}\
.hd p{font-size:12px;opacity:.85;margin-top:3px}\
.bd{padding:22px 24px}\
.relay{display:flex;align-items:center;justify-content:space-between;gap:14px}\
.relay .lbl{font-size:14px;font-weight:600}\
.pill{display:inline-flex;align-items:center;gap:7px;font-size:12px;font-weight:700;padding:5px 12px;border-radius:999px;margin-top:4px}\
.pill.on{background:#dcfce7;color:#15803d}\
.pill.off{background:#fee2e2;color:#b91c1c}\
.dot{width:7px;height:7px;border-radius:50%;background:currentColor}\
.btns{display:flex;gap:9px}\
button{font:inherit;font-weight:600;font-size:14px;border:0;border-radius:10px;padding:10px 20px;cursor:pointer;transition:background .15s}\
.bon{background:#16a34a;color:#fff}\
.boff{background:#dc2626;color:#fff}\
.sec{font-size:11px;font-weight:700;color:#94a3b8;text-transform:uppercase;letter-spacing:.06em;margin:18px 0 8px}\
.sec.first{margin-top:0}\
label{display:block;font-size:12px;font-weight:600;color:#64748b;margin:12px 0 5px}\
input[type=text],input[type=number]{width:100%;font:inherit;font-size:15px;padding:10px 12px;border:1px solid #cbd5e1;border-radius:10px;background:#f8fafc}\
input:focus{outline:0;border-color:#4f46e5;background:#fff;box-shadow:0 0 0 3px rgba(79,70,229,.15)}\
.row{display:flex;gap:12px}\
.row>div{flex:1}\
.chk{display:flex;align-items:center;gap:9px;margin-top:16px}\
.chk input{width:18px;height:18px;accent-color:#4f46e5}\
.chk label{margin:0;font-size:14px;color:#1e293b}\
.save{width:100%;background:#4f46e5;color:#fff;margin-top:20px;padding:12px}\
.foot{padding:16px 24px;border-top:1px solid #eef2f6}\
.reboot{width:100%;background:#fff;color:#dc2626;border:1px solid #fecaca;padding:11px}\
</style></head><body><div class='wrap'>";

const PAGE_FOOT: &str = "</div></body></html>";

async fn w(socket: &mut TcpSocket<'_>, s: &str) {
    let _ = socket.write_all(s.as_bytes()).await;
}

/// Stream the config + relay-control page directly to the socket. Streaming in
/// flash-resident chunks avoids building a multi-KB HTML buffer on the stack.
async fn send_page(socket: &mut TcpSocket<'_>, cfg: &NetworkConfig) {
    let relay_on = crate::OUTPUTS.get_relay();
    let ip = fmt_ipv4(&cfg.ip);
    let gw = fmt_ipv4(&cfg.gateway);
    let bk = fmt_ipv4(&cfg.broker_ip);
    let port = fmt_u16(cfg.broker_port);
    let prefix = fmt_u8(cfg.prefix_len);

    w(socket, HTTP_OK).await;
    w(socket, PAGE_HEAD).await;

    // Header + relay control
    w(socket, "<div class='card'><div class='hd'><h1>JZF407VET6 Controller</h1><p>STM32F407 · Embassy · MQTT</p></div><div class='bd'><div class='relay'><div><div class='lbl'>Relay</div>").await;
    if relay_on {
        w(socket, "<span class='pill on'><span class='dot'></span>ON</span>").await;
    } else {
        w(socket, "<span class='pill off'><span class='dot'></span>OFF</span>").await;
    }
    w(socket, "</div><div class='btns'><form method='post' action='/relay/on'><button class='bon'>ON</button></form><form method='post' action='/relay/off'><button class='boff'>OFF</button></form></div></div></div></div>").await;

    // Config form
    w(socket, "<div class='card'><div class='bd'><form method='post' action='/save'><div class='sec first'>Network</div><label>IP address</label><input type='text' name='ip' value='").await;
    w(socket, ip.as_str()).await;
    w(socket, "'><div class='row'><div><label>Prefix</label><input type='number' name='prefix' value='").await;
    w(socket, prefix.as_str()).await;
    w(socket, "'></div><div><label>Gateway</label><input type='text' name='gw' value='").await;
    w(socket, gw.as_str()).await;
    w(socket, "'></div></div><div class='sec'>MQTT Broker</div><div class='row'><div><label>Broker IP</label><input type='text' name='broker' value='").await;
    w(socket, bk.as_str()).await;
    w(socket, "'></div><div><label>Port</label><input type='number' name='port' value='").await;
    w(socket, port.as_str()).await;
    w(socket, "'></div></div><label>Client ID</label><input type='text' name='id' value='").await;
    w(socket, cfg.client_id.as_str()).await;
    w(socket, "'><div class='chk'><input type='checkbox' id='dhcp' name='dhcp' value='1'").await;
    if cfg.dhcp {
        w(socket, " checked").await;
    }
    w(socket, "><label for='dhcp'>Enable DHCP</label></div><button class='save'>Save Settings</button></form></div><div class='foot'><form method='post' action='/reboot'><button class='reboot'>Reboot Device</button></form></div></div>").await;

    w(socket, PAGE_FOOT).await;
    let _ = socket.flush().await;
}

fn fmt_ipv4(addr: &[u8; 4]) -> HString<16> {
    let mut s: HString<16> = HString::new();
    push_u8_dec(&mut s, addr[0]);
    let _ = s.push('.');
    push_u8_dec(&mut s, addr[1]);
    let _ = s.push('.');
    push_u8_dec(&mut s, addr[2]);
    let _ = s.push('.');
    push_u8_dec(&mut s, addr[3]);
    s
}

fn fmt_u8(v: u8) -> HString<4> {
    let mut s: HString<4> = HString::new();
    push_u8_dec(&mut s, v);
    s
}

fn fmt_u16(v: u16) -> HString<6> {
    let mut s: HString<6> = HString::new();
    push_u16_dec(&mut s, v);
    s
}

fn push_u8_dec<const N: usize>(buf: &mut HString<N>, v: u8) {
    use core::fmt::Write as FmtWrite;
    let _ = write!(buf, "{}", v);
}

fn push_u16_dec<const N: usize>(buf: &mut HString<N>, v: u16) {
    use core::fmt::Write as FmtWrite;
    let _ = write!(buf, "{}", v);
}

#[embassy_executor::task]
pub async fn web_task(stack: Stack<'static>, cfg: NetworkConfig) {
    info!("WEB: waiting for link...");
    stack.wait_link_up().await;
    info!("WEB: link up");
    stack.wait_config_up().await;
    info!("WEB: listening on :80");

    static RX_BUF: static_cell::StaticCell<[u8; 1536]> = static_cell::StaticCell::new();
    static TX_BUF: static_cell::StaticCell<[u8; 4096]> = static_cell::StaticCell::new();
    static REQ_BUF: static_cell::StaticCell<[u8; 1536]> = static_cell::StaticCell::new();
    let rx_buf = RX_BUF.init([0u8; 1536]);
    let tx_buf = TX_BUF.init([0u8; 4096]);
    let req_buf = REQ_BUF.init([0u8; 1536]);

    loop {
        let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if socket.accept(80).await.is_err() {
            Timer::after(Duration::from_millis(100)).await;
            continue;
        }

        let n = match socket.read(req_buf).await {
            Ok(n) if n > 0 => n,
            _ => continue,
        };

        let req = match core::str::from_utf8(&req_buf[..n]) {
            Ok(s) => s,
            Err(_) => {
                let _ = socket.write_all(HTTP_400.as_bytes()).await;
                continue;
            }
        };

        let first_line = req.lines().next().unwrap_or("");
        let mut parts = first_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("/");

        match (method, path) {
            ("GET", "/") => {
                send_page(&mut socket, &cfg).await;
            }
            ("POST", "/relay/on") | ("POST", "/relay/off") => {
                let on = path.ends_with("on");
                crate::OUTPUTS.set_relay(on);
                crate::persistence::save_relay(on).await;
                // Tell mqtt_task to publish the new state to the broker.
                crate::mqtt::RELAY_CHANGE.signal(on);
                info!("WEB: relay {}", if on { "ON" } else { "OFF" });
                let _ = socket.write_all(HTTP_303.as_bytes()).await;
                let _ = socket.flush().await;
            }
            ("POST", "/reboot") => {
                let _ = socket.write_all(HTTP_OK.as_bytes()).await;
                let _ = socket
                    .write_all(b"<html><head><meta http-equiv=refresh content='4;url=/'></head><body><p>Rebooting... <a href=/>back in 4s</a></p></body></html>")
                    .await;
                let _ = socket.flush().await;
                socket.close();
                Timer::after(Duration::from_millis(300)).await;
                warn!("WEB: /reboot triggered sys_reset");
                crate::fault_marker::safe_reboot();
            }
            ("POST", "/save") => {
                if let Some(body) = req.find("\r\n\r\n").map(|i| &req[i + 4..]) {
                    match parse_form(body, &cfg) {
                        Some(new_cfg) => {
                            let _ = socket.write_all(HTTP_OK.as_bytes()).await;
                            let _ = socket
                                .write_all(b"<html><head><meta http-equiv=refresh content='4;url=/'></head><body><p>Saved. Rebooting... <a href=/>back in 4s</a></p></body></html>")
                                .await;
                            let _ = socket.flush().await;
                            socket.close();
                            Timer::after(Duration::from_millis(200)).await;
                            if crate::config::save_config(&new_cfg).await.is_err() {
                                warn!("WEB: save failed");
                            }
                            warn!("WEB: /save triggered sys_reset");
                            crate::fault_marker::safe_reboot();
                        }
                        None => {
                            let _ = socket.write_all(HTTP_400.as_bytes()).await;
                        }
                    }
                } else {
                    let _ = socket.write_all(HTTP_400.as_bytes()).await;
                }
            }
            _ => {
                let _ = socket.write_all(HTTP_404.as_bytes()).await;
            }
        }
    }
}

fn parse_form(body: &str, current: &NetworkConfig) -> Option<NetworkConfig> {
    let mut cfg = current.clone();
    for pair in body.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("").trim();
        let val = kv.next().unwrap_or("").trim();
        let val = url_decode(val);

        match key {
            "ip" => {
                cfg.ip = parse_ipv4(&val)?;
            }
            "prefix" => {
                cfg.prefix_len = val.parse::<u8>().ok()?;
            }
            "gw" => {
                cfg.gateway = parse_ipv4(&val)?;
            }
            "broker" => {
                cfg.broker_ip = parse_ipv4(&val)?;
            }
            "port" => {
                cfg.broker_port = parse_port(&val)?;
            }
            "id" => {
                cfg.client_id = HString::try_from(val.as_str()).ok()?;
            }
            "dhcp" => {
                cfg.dhcp = val == "1";
            }
            _ => {}
        }
    }
    Some(cfg)
}

fn url_decode(s: &str) -> HString<64> {
    let mut out = HString::<64>::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'+' {
            let _ = out.push(' ');
            i += 1;
        } else if c == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                let byte = (h << 4) | l;
                // Only single-byte UTF-8 (ASCII) is representable here; matches prior behavior.
                if byte < 0x80 {
                    let _ = out.push(byte as char);
                }
                i += 3;
            } else {
                let _ = out.push(c as char);
                i += 1;
            }
        } else {
            let _ = out.push(c as char);
            i += 1;
        }
    }
    out
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
