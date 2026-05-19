use defmt::{info, warn};
use embassy_net::{tcp::TcpSocket, Stack};
use embedded_io_async::Write;
use embassy_time::{Duration, Timer};
use heapless::String as HString;
use jzf407_logic::config::{parse_ipv4, parse_port, NetworkConfig};

const HTTP_OK: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n";
const HTTP_400: &str = "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nBad Request";
const HTTP_404: &str = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\nNot Found";

/// Render HTML form with current config values.
fn render_form(cfg: &NetworkConfig) -> HString<2048> {
    let mut h: HString<2048> = HString::new();
    let ip = fmt_ipv4(&cfg.ip);
    let gw = fmt_ipv4(&cfg.gateway);
    let bk = fmt_ipv4(&cfg.broker_ip);
    let port = fmt_u16(cfg.broker_port);
    let prefix = fmt_u8(cfg.prefix_len);
    let dhcp_checked = if cfg.dhcp { " checked" } else { "" };

    // Build HTML by parts — avoiding literal & in strings
    let _ = h.push_str("<!DOCTYPE html><html><head><title>JZF407VET6</title>");
    let _ = h.push_str("<style>body{font:14px monospace;max-width:420px;margin:2em auto;border:1px solid #ccc;padding:1em}label{display:block;margin-top:0.8em}input{margin-bottom:0.4em}</style>");
    let _ = h.push_str("</head><body><h2>JZF407VET6</h2>");
    let _ = h.push_str("<form method=POST action=/save>");

    // IP
    let _ = h.push_str("<label>IP:</label><input name=ip size=16 value='");
    let _ = h.push_str(ip.as_str());
    let _ = h.push_str("'>");

    // Prefix
    let _ = h.push_str("<label>Prefix:</label><input name=prefix size=4 value='");
    let _ = h.push_str(prefix.as_str());
    let _ = h.push_str("'>");

    // Gateway
    let _ = h.push_str("<label>Gateway:</label><input name=gw size=16 value='");
    let _ = h.push_str(gw.as_str());
    let _ = h.push_str("'>");

    // Broker
    let _ = h.push_str("<label>Broker:</label><input name=broker size=16 value='");
    let _ = h.push_str(bk.as_str());
    let _ = h.push_str("'>");

    // Port
    let _ = h.push_str("<label>Port:</label><input name=port size=6 value='");
    let _ = h.push_str(port.as_str());
    let _ = h.push_str("'>");

    // Client ID
    let _ = h.push_str("<label>ID:</label><input name=id size=24 value='");
    let _ = h.push_str(cfg.client_id.as_str());
    let _ = h.push_str("'>");

    // DHCP
    let _ = h.push_str("<label>DHCP:</label><input type=checkbox name=dhcp value=1");
    let _ = h.push_str(dhcp_checked);
    let _ = h.push_str(">");

    let _ = h.push_str("<br><input type=submit value=Save>");
    let _ = h.push_str("</form><form method=POST action=/reboot>");
    let _ = h.push_str("<input type=submit value=Reboot></form>");
    let _ = h.push_str("</body></html>");
    h
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
    stack.wait_config_up().await;
    info!("WEB: listening on :80");

    let mut rx_buf = [0u8; 1536];
    let mut tx_buf = [0u8; 4096]; // HTML up to 2048 + headers ~60 bytes, needs > 2048
    let mut req_buf = [0u8; 1536];

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buf, &mut tx_buf);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if socket.accept(80).await.is_err() {
            Timer::after(Duration::from_millis(100)).await;
            continue;
        }

        let n = match socket.read(&mut req_buf).await {
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
                let html = render_form(&cfg);
                let _ = socket.write_all(HTTP_OK.as_bytes()).await;
                let _ = socket.write_all(html.as_bytes()).await;
                let _ = socket.flush().await;
            }
            ("POST", "/reboot") => {
                let _ = socket.write_all(HTTP_OK.as_bytes()).await;
                let _ = socket
                    .write_all(b"<html><body><p>Rebooting</p></body></html>")
                    .await;
                let _ = socket.flush().await;
                Timer::after(Duration::from_millis(500)).await;
                cortex_m::peripheral::SCB::sys_reset();
            }
            ("POST", "/save") => {
                if let Some(body) = req.find("\r\n\r\n").map(|i| &req[i + 4..]) {
                    match parse_form(body, &cfg) {
                        Some(new_cfg) => {
                            let _ = socket.write_all(HTTP_OK.as_bytes()).await;
                            let _ = socket
                                .write_all(b"<html><body><p>Saved. Rebooting</p></body></html>")
                                .await;
                            let _ = socket.flush().await;
                            Timer::after(Duration::from_millis(200)).await;
                            if crate::config::save_config(&new_cfg).await.is_err() {
                                warn!("WEB: save failed");
                            }
                            cortex_m::peripheral::SCB::sys_reset();
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
                if let Ok(ch) = core::str::from_utf8(&buffer(&[byte])[..]) {
                    for cc in ch.chars() {
                        let _ = out.push(cc);
                    }
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

fn buffer<const N: usize>(data: &[u8; N]) -> [u8; N] {
    *data
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
