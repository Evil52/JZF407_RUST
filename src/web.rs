use crate::fault_marker::ResetReason;
use core::sync::atomic::{AtomicU32, Ordering};
use defmt::{info, warn};
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::Write;
use heapless::String as HString;
use jzf407_logic::config::{parse_ipv4, parse_port, NetworkConfig};

const HTTP_OK: &str =
    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n";
// JSON response for /state polling. `no-store` stops the browser caching it, so
// each poll reflects the live pin state.
const HTTP_OK_JSON: &str =
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n";
// Cookie clearing uses Max-Age=0 + a past Expires for max browser compatibility.
// Path=/ must match the path the cookie was set with, or the browser keeps it.
// Redirect to the dashboard (used when an authenticated client hits /login).
const HTTP_303_HOME: &str = "HTTP/1.1 303 See Other\r\nLocation: /\r\nConnection: close\r\n\r\n";

// Logout: clear the cookie and redirect to the login form.
const HTTP_303_LOGOUT: &str = "HTTP/1.1 303 See Other\r\n\
Set-Cookie: jzf_session=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; SameSite=Strict\r\n\
Location: /login\r\nConnection: close\r\n\r\n";
const HTTP_400: &str = "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nBad Request";
const HTTP_404: &str = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\nNot Found";

/// Login form page parts. Split so the error message can be conditionally
/// included (shown on failed POST /login, hidden on GET / or expired session).
const LOGIN_HEAD: &str = "<!DOCTYPE html><html lang='en'><head><meta charset='utf-8'>\
<meta name='viewport' content='width=device-width,initial-scale=1'><title>JZF407 — Login</title>\
<style>\
*{box-sizing:border-box;margin:0;padding:0}\
body{font-family:system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;color:#e2e8f0;display:flex;align-items:center;justify-content:center;min-height:100vh;padding:16px;\
background:radial-gradient(1000px 600px at 80% -10%,rgba(167,139,250,.2),transparent),radial-gradient(800px 500px at -10% 10%,rgba(34,211,238,.15),transparent),#0a0e1a}\
.login-box{background:rgba(20,27,45,.72);backdrop-filter:blur(12px);-webkit-backdrop-filter:blur(12px);border:1px solid rgba(148,163,184,.14);border-radius:18px;box-shadow:0 10px 40px rgba(0,0,0,.5);padding:40px 32px;max-width:380px;width:100%;text-align:center}\
.login-box h1{font-size:24px;font-weight:700;margin-bottom:6px;background:linear-gradient(90deg,#22d3ee,#a78bfa);-webkit-background-clip:text;background-clip:text;-webkit-text-fill-color:transparent}\
.login-box .sub{color:#64748b;font-size:14px;margin-bottom:24px}\
.login-box .err{background:rgba(251,113,133,.12);color:#fb7185;border:1px solid rgba(251,113,133,.3);border-radius:10px;padding:10px 14px;font-size:13px;font-weight:600;margin-bottom:16px}\
.login-box input{display:block;width:100%;font:inherit;font-size:15px;padding:12px 14px;border:1px solid rgba(148,163,184,.18);border-radius:10px;background:rgba(10,14,26,.6);color:#e2e8f0;margin-bottom:12px}\
.login-box input:focus{outline:0;border-color:#22d3ee;background:rgba(10,14,26,.9);box-shadow:0 0 0 3px rgba(34,211,238,.18)}\
.login-box input::placeholder{color:#475569}\
.login-box button{width:100%;font:inherit;font-weight:700;font-size:15px;border:0;border-radius:10px;padding:13px;cursor:pointer;color:#0a0e1a;background:linear-gradient(90deg,#22d3ee,#a78bfa);transition:filter .15s,transform .12s}\
.login-box button:hover{filter:brightness(1.12)}.login-box button:active{transform:scale(.98)}\
</style></head><body>\
<div class='login-box'><h1>JZF407</h1>\
<p class='sub'>Enter your credentials to continue</p>";

/// Error message shown inside the login form on a failed login attempt.
const LOGIN_ERROR: &str = "<div class='err'>Invalid username or password</div>";

/// Login form (after the optional error message).
const LOGIN_TAIL: &str = "<form method='post' action='/login'>\
<input type='text' name='user' placeholder='Username' autocomplete='username' required>\
<input type='password' name='pass' placeholder='Password' autocomplete='current-password' required>\
<button type='submit'>Sign In</button>\
</form></div></body></html>";

// 401 + login form. No WWW-Authenticate header: the browser must render our
// styled HTML form, never its native Basic Auth popup.
const HTTP_401_LOGIN: &str = "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n";

// Same as above but also expires a stale session cookie in the response.
const HTTP_401_EXPIRE_COOKIE: &str = "HTTP/1.1 401 Unauthorized\r\n\
Set-Cookie: jzf_session=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; SameSite=Strict\r\n\
Content-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n";

const PAGE_HEAD: &str = "<!DOCTYPE html><html lang='en'><head><meta charset='utf-8'>\
<meta name='viewport' content='width=device-width,initial-scale=1'><title>JZF407VET6</title>\
<style>\
*{box-sizing:border-box;margin:0;padding:0}\
:root{--cy:#22d3ee;--vi:#a78bfa;--ok:#34d399;--no:#fb7185;--mut:#64748b;--bd:rgba(148,163,184,.14)}\
body{font-family:system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;color:#e2e8f0;line-height:1.5;padding:22px 14px;min-height:100vh;\
background:radial-gradient(1200px 600px at 80% -10%,rgba(167,139,250,.18),transparent),radial-gradient(900px 500px at -10% 10%,rgba(34,211,238,.14),transparent),#0a0e1a}\
.wrap{max-width:920px;margin:0 auto}\
.grid{display:grid;grid-template-columns:1fr 1fr;gap:16px;align-items:start}\
.col{display:flex;flex-direction:column}\
@media(max-width:720px){.grid{grid-template-columns:1fr}}\
.card{background:rgba(20,27,45,.72);backdrop-filter:blur(12px);-webkit-backdrop-filter:blur(12px);border:1px solid var(--bd);border-radius:18px;box-shadow:0 10px 40px rgba(0,0,0,.45);margin-bottom:16px;overflow:hidden}\
.col .card:last-child{margin-bottom:0}\
.hd{position:relative;padding:20px 22px;border-bottom:1px solid var(--bd);background:linear-gradient(135deg,rgba(34,211,238,.12),rgba(167,139,250,.12))}\
.hd h1{font-size:17px;font-weight:700;letter-spacing:.3px;display:flex;align-items:center;gap:9px}\
.hd h1 .led{width:9px;height:9px;border-radius:50%;background:var(--cy);box-shadow:0 0 10px var(--cy);animation:pulse 2s infinite}\
@keyframes pulse{0%,100%{opacity:1}50%{opacity:.4}}\
.hd p{font-size:11px;color:var(--mut);margin-top:3px;letter-spacing:.4px}\
.clk{margin-top:14px;display:flex;align-items:baseline;gap:10px}\
.clk #clk{font-size:34px;font-weight:700;font-variant-numeric:tabular-nums;letter-spacing:1px;background:linear-gradient(90deg,var(--cy),var(--vi));-webkit-background-clip:text;background-clip:text;-webkit-text-fill-color:transparent;text-shadow:0 0 24px rgba(34,211,238,.25)}\
.clk #dt{font-size:12px;color:var(--mut);font-weight:600}\
.bd{padding:18px 22px}\
.relay{display:flex;align-items:center;justify-content:space-between;gap:14px}\
.relay .lbl{font-size:14px;font-weight:600}\
.pill{display:inline-flex;align-items:center;gap:7px;font-size:12px;font-weight:700;padding:5px 12px;border-radius:999px}\
.pill.on{background:rgba(52,211,153,.14);color:var(--ok);box-shadow:0 0 0 1px rgba(52,211,153,.3),0 0 14px rgba(52,211,153,.2)}\
.pill.off{background:rgba(251,113,133,.12);color:var(--no);box-shadow:0 0 0 1px rgba(251,113,133,.25)}\
.dot{width:7px;height:7px;border-radius:50%;background:currentColor;box-shadow:0 0 8px currentColor}\
.leds{display:flex;gap:8px;margin-top:14px}\
.kv{display:flex;justify-content:space-between;align-items:center;padding:10px 0;border-bottom:1px solid var(--bd);font-size:13px}\
.kv:last-child{border-bottom:0}\
.k{color:var(--mut);font-weight:600}\
.v{font-weight:600;font-variant-numeric:tabular-nums;color:#cbd5e1}\
button{font:inherit;font-weight:600;font-size:14px;border:0;border-radius:11px;padding:11px 20px;cursor:pointer;transition:transform .12s,filter .15s}\
button:active{transform:scale(.97)}button:hover{filter:brightness(1.12)}\
.sec{font-size:10px;font-weight:700;color:var(--mut);text-transform:uppercase;letter-spacing:.1em;margin:18px 0 8px}\
.sec.first{margin-top:0}\
label{display:block;font-size:12px;font-weight:600;color:var(--mut);margin:12px 0 5px}\
input[type=text],input[type=number],input[type=password]{width:100%;font:inherit;font-size:15px;padding:10px 12px;border:1px solid var(--bd);border-radius:10px;background:rgba(10,14,26,.6);color:#e2e8f0}\
input:focus{outline:0;border-color:var(--cy);background:rgba(10,14,26,.9);box-shadow:0 0 0 3px rgba(34,211,238,.18)}\
.row{display:flex;gap:12px}\
.row>div{flex:1}\
.save{width:100%;color:#0a0e1a;margin-top:20px;padding:13px;background:linear-gradient(90deg,var(--cy),var(--vi))}\
.foot{padding:16px 22px;border-top:1px solid var(--bd);display:flex;flex-direction:column;gap:9px}\
.reboot{width:100%;background:rgba(251,113,133,.1);color:var(--no);border:1px solid rgba(251,113,133,.3)}\
.logout{width:100%;background:transparent;color:var(--cy);border:1px solid rgba(34,211,238,.35)}\
.toast{position:fixed;bottom:20px;left:50%;transform:translateX(-50%);background:rgba(20,27,45,.95);border:1px solid var(--bd);color:#e2e8f0;padding:11px 24px;border-radius:12px;font-size:13px;font-weight:600;z-index:999;opacity:0;transition:opacity .3s;box-shadow:0 8px 30px rgba(0,0,0,.5)}\
.toast.show{opacity:1}\
</style></head><body><div class='wrap'>";

// Footer with:
// - Inactivity-based auto-logout (15 min idle → POST /logout to invalidate server session)
// - Logout button handler (POST /logout)
// - Toast notification for logout warning
// - The existing live-polling script
const PAGE_FOOT: &str = "<script>\
function g(i){return document.getElementById(i)}\
function pl(i,o,t){var e=g(i);if(e){e.className='pill '+(o?'on':'off');e.innerHTML=\"<span class='dot'></span>\"+t}}\
function tx(i,v){var e=g(i);if(e){e.textContent=v}}\
function z(n){return(n<10?'0':'')+n}\
function fu(n){var d=n/86400|0,h=n%86400/3600|0,m=n%3600/60|0,s=n%60;return (d?d+'d ':'')+z(h)+':'+z(m)+':'+z(s)}\
var DAYS=['Sun','Mon','Tue','Wed','Thu','Fri','Sat'];\
var MON=['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec'];\
function clock(){var n=new Date();tx('clk',z(n.getHours())+':'+z(n.getMinutes())+':'+z(n.getSeconds()));tx('dt',DAYS[n.getDay()]+', '+n.getDate()+' '+MON[n.getMonth()]+' '+n.getFullYear())}\
var loggingOut=false;\
function toLogin(){if(loggingOut)return;loggingOut=true;location.href='/login'}\
var IDLE_TIMEOUT=900,lastActivity=Date.now();\
var upBase=0,upAt=0;\
function p(){if(loggingOut)return;var a=(Date.now()-lastActivity)/1000<30?'?active=1':'';fetch('/state'+a,{credentials:'same-origin'}).then(function(r){if(r.status===401){toLogin();return null}return r.ok?r.json():null}).then(function(s){if(!s)return;pl('rp',s.relay,s.relay?'ON':'OFF');pl('l1',s.led1,'LED1');pl('l2',s.led2,'LED2');pl('lk',s.link,s.link?'Up':'Down');pl('mq',s.mqtt,s.mqtt?'Online':'Offline');tx('ip',s.ip);tx('rst',s.rst);upBase=s.up;upAt=Date.now()}).catch(function(){})}\
function resetIdle(){lastActivity=Date.now();if(g('toast'))g('toast').className='toast'}\
['mousemove','keydown','touchstart','scroll','click'].forEach(function(e){document.addEventListener(e,resetIdle,{passive:true})});\
function doLogout(){if(loggingOut)return;loggingOut=true;var f=document.createElement('form');f.method='post';f.action='/logout';document.body.appendChild(f);f.submit()}\
setInterval(function(){\
clock();\
if(upAt)tx('up',fu(upBase+Math.floor((Date.now()-upAt)/1000)));\
if(loggingOut)return;\
var idle=(Date.now()-lastActivity)/1000;\
if(idle>=IDLE_TIMEOUT){doLogout()}\
else if(idle>=IDLE_TIMEOUT-60&&g('toast')){g('toast').className='toast show';g('toast').textContent='Session expires in '+(IDLE_TIMEOUT-Math.floor(idle))+'s — move mouse to stay'}\
},1000);\
clock();setInterval(p,1000);p();\
</script><div id='toast' class='toast'></div></div></body></html>";

async fn w(socket: &mut TcpSocket<'_>, s: &str) {
    let _ = socket.write_all(s.as_bytes()).await;
}

/// Stream one status pill: `<span class='pill on|off' id=...><dot>TEXT</span>`.
/// `id` lets the client-side poller find and repaint it (see PAGE_FOOT script).
async fn pill(socket: &mut TcpSocket<'_>, id: &str, on: bool, text: &str) {
    w(socket, "<span class='pill ").await;
    w(socket, if on { "on" } else { "off" }).await;
    w(socket, "' id='").await;
    w(socket, id).await;
    w(socket, "'><span class='dot'></span>").await;
    w(socket, text).await;
    w(socket, "</span>").await;
}

/// Live state as compact JSON for the polling script, e.g.
/// `{"relay":1,"led1":0,"led2":0,"link":1,"mqtt":1,"up":3661,"ip":"192.168.137.2","rst":"power_on"}`.
/// Outputs are read from the physical pins (single source of truth); `up` is
/// seconds since boot (formatted client-side). Intentionally logs nothing — it is
/// hit ~once per second per open page, so logging here would spam the RTT.
async fn send_state(
    socket: &mut TcpSocket<'_>,
    cfg: &NetworkConfig,
    stack: Stack<'static>,
    reset: ResetReason,
) {
    use core::fmt::Write as _;
    let ip = fmt_ipv4(&cfg.ip);
    let mut body: HString<160> = HString::new();
    let _ = write!(
        body,
        "{{\"relay\":{},\"led1\":{},\"led2\":{},\"link\":{},\"mqtt\":{},\"up\":{},\"ip\":\"{}\",\"rst\":\"{}\"}}",
        crate::OUTPUTS.get_relay() as u8,
        crate::OUTPUTS.get_led(crate::LedId::Led1) as u8,
        crate::OUTPUTS.get_led(crate::LedId::Led2) as u8,
        stack.is_link_up() as u8,
        crate::mqtt::MQTT_ONLINE.load(Ordering::Relaxed) as u8,
        Instant::now().as_secs(),
        ip.as_str(),
        reset.as_str(),
    );
    w(socket, HTTP_OK_JSON).await;
    w(socket, body.as_str()).await;
    let _ = socket.flush().await;
}

/// Stream the config + relay-control page directly to the socket. Streaming in
/// flash-resident chunks avoids building a multi-KB HTML buffer on the stack.
async fn send_page(
    socket: &mut TcpSocket<'_>,
    cfg: &NetworkConfig,
    stack: Stack<'static>,
    reset: ResetReason,
) {
    let relay_on = crate::OUTPUTS.get_relay();
    let led1_on = crate::OUTPUTS.get_led(crate::LedId::Led1);
    let led2_on = crate::OUTPUTS.get_led(crate::LedId::Led2);
    let ip = fmt_ipv4(&cfg.ip);
    let gw = fmt_ipv4(&cfg.gateway);
    let bk = fmt_ipv4(&cfg.broker_ip);
    let port = fmt_u16(cfg.broker_port);
    let prefix = fmt_u8(cfg.prefix_len);

    w(socket, HTTP_OK).await;
    w(socket, PAGE_HEAD).await;

    // Header card: title + live clock/date (filled by JS from the browser's own
    // Date(), so it ticks every second with no MCU clock and no page reload).
    // The pill ids (rp/l1/l2) are the hooks the polling script repaints; initial
    // classes below just avoid a flash before first poll.
    // Relay has no buttons — controlled via MQTT only (stm32/relay).
    // Full-width header, then a 2-column grid (collapses to 1 col on phones).
    w(socket, "<div class='card'><div class='hd'><h1><span class='led'></span>JZF407VET6</h1><p>STM32F407 · Embassy · MQTT</p><div class='clk'><span id='clk'>--:--:--</span><span id='dt'></span></div></div></div>").await;

    // ---- Left column: Relay/LED + Status ----
    w(socket, "<div class='grid'><div class='col'><div class='card'><div class='bd'><div class='relay'><div><div class='lbl'>Relay (MQTT only)</div>").await;
    pill(socket, "rp", relay_on, if relay_on { "ON" } else { "OFF" }).await;
    w(socket, "</div></div><div class='leds'>").await;
    pill(socket, "l1", led1_on, "LED1").await;
    pill(socket, "l2", led2_on, "LED2").await;
    w(socket, "</div></div></div>").await;

    // Live telemetry card. lk/mq are pills; ip/up/rst are text repainted by the
    // poller. Initial values are real (except uptime, which the first poll fills).
    let link_up = stack.is_link_up();
    let mqtt_up = crate::mqtt::MQTT_ONLINE.load(Ordering::Relaxed);
    w(socket, "<div class='card'><div class='bd'><div class='sec first'>Status</div><div class='kv'><span class='k'>Link</span>").await;
    pill(socket, "lk", link_up, if link_up { "Up" } else { "Down" }).await;
    w(socket, "</div><div class='kv'><span class='k'>MQTT</span>").await;
    pill(
        socket,
        "mq",
        mqtt_up,
        if mqtt_up { "Online" } else { "Offline" },
    )
    .await;
    w(
        socket,
        "</div><div class='kv'><span class='k'>IP</span><span class='v' id='ip'>",
    )
    .await;
    w(socket, ip.as_str()).await;
    w(socket, "</span></div><div class='kv'><span class='k'>Uptime</span><span class='v' id='up'>—</span></div><div class='kv'><span class='k'>Last reset</span><span class='v' id='rst'>").await;
    w(socket, reset.as_str()).await;
    // Close Status card + left column, open right column for the config form.
    w(socket, "</span></div></div></div></div><div class='col'>").await;

    // ---- Right column: config form ----
    w(socket, "<div class='card'><div class='bd'><form method='post' action='/save'><div class='sec first'>Network</div><label>IP address</label><input type='text' name='ip' value='").await;
    w(socket, ip.as_str()).await;
    w(
        socket,
        "'><div class='row'><div><label>Prefix</label><input type='number' name='prefix' value='",
    )
    .await;
    w(socket, prefix.as_str()).await;
    w(
        socket,
        "'></div><div><label>Gateway</label><input type='text' name='gw' value='",
    )
    .await;
    w(socket, gw.as_str()).await;
    w(socket, "'></div></div><div class='sec'>MQTT Broker</div><div class='row'><div><label>Broker IP</label><input type='text' name='broker' value='").await;
    w(socket, bk.as_str()).await;
    w(
        socket,
        "'></div><div><label>Port</label><input type='number' name='port' value='",
    )
    .await;
    w(socket, port.as_str()).await;
    w(
        socket,
        "'></div></div><label>Client ID</label><input type='text' name='id' value='",
    )
    .await;
    w(socket, cfg.client_id.as_str()).await;

    // MQTT broker credentials (sent in the CONNECT packet). Empty = anonymous.
    w(socket, "'><div class='sec'>MQTT Auth</div><div class='row'><div><label>Username</label><input type='text' name='muser' value='").await;
    w(socket, cfg.mqtt_user.as_str()).await;
    w(
        socket,
        "'></div><div><label>Password</label><input type='password' name='mpass' value='",
    )
    .await;
    w(socket, cfg.mqtt_pass.as_str()).await;

    // Web login (form + session cookie). Leaving BOTH blank disables the login.
    w(socket, "'></div></div><div class='sec'>Web Login</div><div class='row'><div><label>Username</label><input type='text' name='wuser' value='").await;
    w(socket, cfg.web_user.as_str()).await;
    w(
        socket,
        "'></div><div><label>Password</label><input type='password' name='wpass' value='",
    )
    .await;
    w(socket, cfg.web_pass.as_str()).await;

    // Close form/.bd + .card, then .foot card, then right .col and the .grid.
    w(socket, "'></div></div><button class='save'>Save Settings</button></form></div><div class='foot'><form method='post' action='/reboot'><button class='reboot'>Reboot Device</button></form><button class='logout' onclick='doLogout()'>Log Out</button></div></div></div></div>").await;

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

/// Global session state: a 64-bit token (two 32-bit atomics: top/bottom halves)
/// plus the uptime-seconds it was last refreshed. POST /login validates the
/// submitted credentials, mints this token, and sets it as the `jzf_session`
/// cookie. Every later request is authenticated by that cookie alone — there is
/// no HTTP Basic Auth (browsers cache and auto-resend Basic credentials, which
/// makes logout impossible). Logout clears the token here and expires the cookie.
/// A single global session means one logged-in client at a time (fine for this
/// device); a new login supersedes the previous one.
static SESSION_TOKEN_HI: AtomicU32 = AtomicU32::new(0);
static SESSION_TOKEN_LO: AtomicU32 = AtomicU32::new(0);
static SESSION_CREATED: AtomicU32 = AtomicU32::new(0);

const SESSION_TTL_SECS: u32 = 900; // 15 minutes idle timeout

/// Mint a new session token (simple LCG-based pseudo-random, sufficient here).
fn mint_session_token(seed: u64) -> u64 {
    seed.wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
}

/// Validate the session cookie from a request. Returns `true` if the cookie is
/// present, matches the live token, and is within the idle TTL.
///
/// `refresh` controls the sliding window: real navigation (GET /, POST /save…)
/// passes `true` to extend the TTL; the background /state poller passes `false`,
/// so an idle tab can't keep the session alive forever just by polling. This is
/// what makes the server-side auto-logout reliable even if the JS timer fails.
fn validate_session(req: &str, refresh: bool) -> bool {
    let token_hi = SESSION_TOKEN_HI.load(Ordering::Relaxed);
    let token_lo = SESSION_TOKEN_LO.load(Ordering::Relaxed);
    let token = ((token_hi as u64) << 32) | (token_lo as u64);

    if token == 0 {
        return false;
    }

    let cookie_token = match extract_cookie(req, "jzf_session") {
        Some(t) => t,
        None => return false,
    };

    // Constant-time-ish comparison: compare as u64 (the cookie value is the decimal
    // representation of the token).
    let parsed: u64 = match core::str::from_utf8(cookie_token)
        .ok()
        .and_then(|s| s.parse().ok())
    {
        Some(v) => v,
        None => return false,
    };

    if parsed != token {
        return false;
    }

    let created = SESSION_CREATED.load(Ordering::Relaxed) as u64;
    let now = Instant::now().as_secs();

    if now.saturating_sub(created) > SESSION_TTL_SECS as u64 {
        // Session expired — clear it
        SESSION_TOKEN_HI.store(0, Ordering::Relaxed);
        SESSION_TOKEN_LO.store(0, Ordering::Relaxed);
        return false;
    }

    // Slide the TTL forward only on real activity (not background polling).
    if refresh {
        SESSION_CREATED.store(now as u32, Ordering::Relaxed);
    }
    true
}

/// Extract a named cookie value from the HTTP Cookie header.
fn extract_cookie<'a>(req: &'a str, name: &str) -> Option<&'a [u8]> {
    const PREFIX: &str = "cookie:";
    for line in req.lines() {
        if line.len() < PREFIX.len() || !line[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
            continue;
        }
        let val = line[PREFIX.len()..].trim();
        for cookie in val.split(';') {
            let cookie = cookie.trim();
            if let Some(rest) = cookie.strip_prefix(name) {
                if let Some(value) = rest.trim_start().strip_prefix('=') {
                    return Some(value.trim().as_bytes());
                }
            }
        }
    }
    None
}

#[embassy_executor::task]
pub async fn web_task(stack: Stack<'static>, cfg: NetworkConfig, reset_reason: ResetReason) {
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

    // HTTP Basic Auth. Enabled only when a web username or password is set; an
    // all-empty pair leaves the page open (matches a fresh/upgraded device on a
    // trusted LAN). The expected base64(user:pass) token is built once here and
    // compared against the Authorization header on every request.
    let auth_required = !cfg.web_user.is_empty() || !cfg.web_pass.is_empty();
    let expected_token =
        jzf407_logic::auth::basic_token(cfg.web_user.as_str(), cfg.web_pass.as_str());
    if auth_required {
        info!("WEB: HTTP Basic Auth enabled");
    } else {
        warn!("WEB: no web password set — page is open to anyone who can reach it");
    }

    // Seed the token generator with the current uptime (monotonic, no RTC needed).
    let token_seed = Instant::now().as_secs().wrapping_mul(0x9E3779B97F4A7C15);

    loop {
        let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);
        socket.set_timeout(Some(Duration::from_secs(10)));
        // Disable Nagle: with ~30 small write_all() calls per page, Nagle + the
        // host's delayed-ACK (40–200 ms) would stall each write until the previous
        // segment is ACKed, making the page take several seconds to load.
        socket.set_nagle_enabled(false);

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

        // Parse method + target. Split the query string off the path so routing
        // and TTL logic see a clean path (e.g. "/state?active=1" → "/state").
        let first_line = req.lines().next().unwrap_or("");
        let mut parts = first_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let target = parts.next().unwrap_or("/");
        let (path, query) = match target.split_once('?') {
            Some((p, q)) => (p, q),
            None => (target, ""),
        };

        // --- Auth gate ---
        // Form-login + session-cookie only (no HTTP Basic Auth). Browsers cache
        // Basic credentials and auto-resend them, which makes logout impossible;
        // a cookie session can be cleared server-side and in the browser, so
        // logout actually works. Auth flow: valid cookie → in; otherwise the
        // /login form mints a cookie on correct credentials.
        //
        // Idle-TTL sliding: real navigation always refreshes it. The /state poll
        // refreshes it ONLY when the page reports recent user activity
        // (?active=1) — so an open-but-abandoned tab can't keep the session alive
        // by polling, but an actively-used page stays logged in.
        let refresh_ttl = path != "/state" || query.contains("active=1");
        let authenticated = !auth_required || validate_session(req, refresh_ttl);

        if !authenticated {
            // POST /login — validate form-submitted credentials (user & pass).
            if method == "POST" && path == "/login" {
                let login_ok = if let Some(body_start) = req.find("\r\n\r\n") {
                    let body = &req[body_start + 4..];
                    let (user_opt, pass_opt) = parse_login_form(body);
                    if let (Some(u), Some(p)) = (user_opt, pass_opt) {
                        let submitted = jzf407_logic::auth::basic_token(u.as_str(), p.as_str());
                        submitted.as_str() == expected_token.as_str()
                    } else {
                        false
                    }
                } else {
                    false
                };

                if login_ok {
                    // Credentials correct — mint session and redirect home.
                    let new_token =
                        mint_session_token(token_seed.wrapping_add(Instant::now().as_secs()));
                    SESSION_TOKEN_HI.store((new_token >> 32) as u32, Ordering::Relaxed);
                    SESSION_TOKEN_LO.store(new_token as u32, Ordering::Relaxed);
                    SESSION_CREATED.store(Instant::now().as_secs() as u32, Ordering::Relaxed);
                    let mut redirect: HString<256> = HString::new();
                    let _ = core::fmt::Write::write_fmt(
                        &mut redirect,
                        format_args!(
                            "HTTP/1.1 303 See Other\r\nSet-Cookie: jzf_session={}; Path=/; Max-Age=1800; HttpOnly; SameSite=Strict\r\nLocation: /\r\nConnection: close\r\n\r\n",
                            new_token
                        ),
                    );
                    w(&mut socket, redirect.as_str()).await;
                    let _ = socket.flush().await;
                    socket.close();
                    continue;
                }
                // Credentials wrong — re-show the login form with an error.
                w(&mut socket, HTTP_401_LOGIN).await;
                w(&mut socket, LOGIN_HEAD).await;
                w(&mut socket, LOGIN_ERROR).await;
                w(&mut socket, LOGIN_TAIL).await;
                let _ = socket.flush().await;
                socket.close();
                continue;
            }

            // Not authenticated and not a login POST: show the login form.
            // If a (now-invalid/expired) session cookie is present, expire it in
            // the same response so the browser drops it. No WWW-Authenticate
            // header anywhere — the browser must never pop its native Basic dialog.
            if extract_cookie(req, "jzf_session").is_some() {
                w(&mut socket, HTTP_401_EXPIRE_COOKIE).await;
            } else {
                w(&mut socket, HTTP_401_LOGIN).await;
            }
            w(&mut socket, LOGIN_HEAD).await;
            w(&mut socket, LOGIN_TAIL).await;
            let _ = socket.flush().await;
            socket.close();
            continue;
        }

        match (method, path) {
            ("GET", "/") => {
                // The session cookie is minted only by POST /login. Here we just
                // render the page; the request already carried a valid cookie
                // (auth gate passed), so no Set-Cookie is needed.
                send_page(&mut socket, &cfg, stack, reset_reason).await;
                // Graceful FIN so the browser gets a clean EOF, not a RST.
                // Without this, drop() calls abort() → RST → fetch('/state')
                // fails with ERR_CONNECTION_RESET and live updates stop working.
                socket.close();
            }
            // Already authenticated but hitting /login (e.g. via back button or
            // login when auth is disabled) — just send them to the dashboard.
            ("GET", "/login") => {
                let _ = socket.write_all(HTTP_303_HOME.as_bytes()).await;
                let _ = socket.flush().await;
                socket.close();
            }
            ("GET", "/state") => {
                send_state(&mut socket, &cfg, stack, reset_reason).await;
                socket.close();
            }
            ("POST", "/logout") => {
                // Clear server-side session first, then send redirect with
                // expired cookie so the browser drops it too.
                SESSION_TOKEN_HI.store(0, Ordering::Relaxed);
                SESSION_TOKEN_LO.store(0, Ordering::Relaxed);
                SESSION_CREATED.store(0, Ordering::Relaxed);
                info!("WEB: user logged out");
                let _ = socket.write_all(HTTP_303_LOGOUT.as_bytes()).await;
                let _ = socket.flush().await;
                socket.close();
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
                socket.close();
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
                // Reject 0 and >32: smoltcp's Ipv4Cidr::new asserts prefix_len<=32
                // and would panic at boot, bricking the board into a reset loop
                // until the EEPROM is wiped. Treat as invalid form (→ 400).
                let p = val.parse::<u8>().ok()?;
                if !(1..=32).contains(&p) {
                    return None;
                }
                cfg.prefix_len = p;
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
            // Credentials: a value longer than the 32-byte field is rejected (→ 400).
            "muser" => {
                cfg.mqtt_user = HString::try_from(val.as_str()).ok()?;
            }
            "mpass" => {
                cfg.mqtt_pass = HString::try_from(val.as_str()).ok()?;
            }
            "wuser" => {
                cfg.web_user = HString::try_from(val.as_str()).ok()?;
            }
            "wpass" => {
                cfg.web_pass = HString::try_from(val.as_str()).ok()?;
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

/// Extract url-decoded `user` and `pass` values from a POST /login form body.
fn parse_login_form(body: &str) -> (Option<HString<64>>, Option<HString<64>>) {
    let mut user = None;
    let mut pass = None;
    for pair in body.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("");
        let val = kv.next().unwrap_or("");
        match key {
            "user" => user = Some(url_decode(val)),
            "pass" => pass = Some(url_decode(val)),
            _ => {}
        }
    }
    (user, pass)
}
