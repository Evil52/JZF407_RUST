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
/// Same cyberpunk theme as the dashboard: HUD-bracketed code-window over the
/// scanline/grain/vignette overlays, JetBrains Mono via CDN with mono fallback.
const LOGIN_HEAD: &str = "<!DOCTYPE html><html lang='en'><head><meta charset='utf-8'>\
<meta name='viewport' content='width=device-width,initial-scale=1'><title>JZF407 // AUTH</title>\
<link rel='preconnect' href='https://fonts.gstatic.com' crossorigin>\
<link href='https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;700;800&display=swap' rel='stylesheet'>\
<style>\
:root{--bg:#05060a;--bg2:#090b14;--ink:#e8f6ff;--dim:#6a7891;--dimmer:#3a445a;--line:#18253b;--line-hot:#1f3a6b;--cy:#00e7ff;--mg:#ff2bd6;--no:#ff6ae3}\
*{box-sizing:border-box;margin:0;padding:0}\
html,body{background:var(--bg);color:var(--ink);font-family:'JetBrains Mono',ui-monospace,Menlo,Consolas,monospace}\
body{display:flex;align-items:center;justify-content:center;min-height:100vh;padding:16px;overflow:hidden;background:radial-gradient(1000px 600px at 80% -10%,rgba(255,43,214,.12),transparent),radial-gradient(800px 500px at -10% 10%,rgba(0,231,255,.12),transparent),var(--bg)}\
.scanlines{position:fixed;inset:0;pointer-events:none;z-index:50;background:repeating-linear-gradient(to bottom,rgba(255,255,255,.025) 0 1px,transparent 1px 3px);mix-blend-mode:overlay;opacity:.6}\
.vignette{position:fixed;inset:0;pointer-events:none;z-index:50;background:radial-gradient(120% 80% at 50% 50%,transparent 55%,rgba(0,0,0,.65) 100%)}\
.box{position:relative;z-index:10;width:100%;max-width:380px;border:1px solid var(--line-hot);background:linear-gradient(180deg,rgba(0,231,255,.04),rgba(0,0,0,0) 40%),var(--bg2);box-shadow:0 0 0 1px rgba(0,231,255,.06),0 30px 80px rgba(0,0,0,.6),inset 0 0 60px rgba(0,231,255,.03)}\
.box::before,.box::after,.box>.bk1,.box>.bk2{content:'';position:absolute;width:12px;height:12px;border:1px solid var(--cy)}\
.box::before{top:-1px;left:-1px;border-right:none;border-bottom:none}\
.box::after{top:-1px;right:-1px;border-left:none;border-bottom:none}\
.box>.bk1{bottom:-1px;left:-1px;border-right:none;border-top:none}\
.box>.bk2{bottom:-1px;right:-1px;border-left:none;border-top:none}\
.chrome{display:flex;align-items:center;justify-content:space-between;padding:12px 18px;border-bottom:1px solid var(--line);font-size:11px;letter-spacing:.24em;text-transform:uppercase;color:var(--dim);background:rgba(0,0,0,.35)}\
.chrome .slash{color:var(--mg)}\
.body{padding:30px 28px}\
.body h1{font-size:30px;font-weight:800;letter-spacing:-.03em;margin-bottom:4px}\
.body h1 .ac{color:transparent;-webkit-text-stroke:1px var(--cy);text-shadow:0 0 18px rgba(0,231,255,.4)}\
.body .sub{color:var(--dim);font-size:11px;letter-spacing:.2em;text-transform:uppercase;margin-bottom:24px}\
.err{color:var(--no);border:1px solid rgba(255,43,214,.4);background:rgba(255,43,214,.07);padding:10px 14px;font-size:11px;font-weight:600;letter-spacing:.1em;text-transform:uppercase;margin-bottom:16px}\
label{display:block;font-size:10px;font-weight:600;color:var(--dim);letter-spacing:.18em;text-transform:uppercase;margin:0 0 5px}\
input{display:block;width:100%;font:inherit;font-size:14px;padding:11px 13px;border:1px solid var(--line-hot);background:rgba(0,0,0,.35);color:var(--ink);margin-bottom:14px}\
input:focus{outline:0;border-color:var(--cy);background:rgba(0,231,255,.04);box-shadow:0 0 0 1px var(--cy),inset 0 0 18px rgba(0,231,255,.06)}\
input::placeholder{color:var(--dimmer)}\
button{position:relative;width:100%;font:inherit;font-weight:600;font-size:12px;letter-spacing:.24em;text-transform:uppercase;border:1px solid var(--mg);padding:14px;cursor:pointer;color:#fff;background:linear-gradient(180deg,rgba(255,43,214,.16),rgba(255,43,214,.04));box-shadow:0 0 22px rgba(255,43,214,.25),inset 0 0 30px rgba(255,43,214,.12);transition:all .2s ease;margin-top:6px}\
button:hover{background:rgba(255,43,214,.24);box-shadow:0 0 26px rgba(255,43,214,.45)}\
button::before,button::after{content:'';position:absolute;width:8px;height:8px;border:1px solid #fff}\
button::before{top:-1px;left:-1px;border-right:none;border-bottom:none}\
button::after{bottom:-1px;right:-1px;border-left:none;border-top:none}\
</style></head><body><div class='scanlines'></div><div class='vignette'></div>\
<div class='box'><span class='bk1'></span><span class='bk2'></span>\
<div class='chrome'><span>auth<span class='slash'>/</span>session</span><span>JZF407</span></div>\
<div class='body'><h1>JZF<span class='ac'>407</span></h1>\
<p class='sub'>// authorization required</p>";

/// Error message shown inside the login form on a failed login attempt.
const LOGIN_ERROR: &str = "<div class='err'>\u{2715} Invalid username or password</div>";

/// Login form (after the optional error message).
const LOGIN_TAIL: &str = "<form method='post' action='/login'>\
<label>Username</label><input type='text' name='user' placeholder='user' autocomplete='username' required>\
<label>Password</label><input type='password' name='pass' placeholder='\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}' autocomplete='current-password' required>\
<button type='submit'>\u{25b8} Authenticate</button>\
</form></div></div></body></html>";

// 401 + login form. No WWW-Authenticate header: the browser must render our
// styled HTML form, never its native Basic Auth popup.
const HTTP_401_LOGIN: &str = "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n";

// Same as above but also expires a stale session cookie in the response.
const HTTP_401_EXPIRE_COOKIE: &str = "HTTP/1.1 401 Unauthorized\r\n\
Set-Cookie: jzf_session=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; SameSite=Strict\r\n\
Content-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n";

// Cyberpunk "SIGNAL.EDGE" theme adapted from the Claude Design mock: JetBrains
// Mono via Google Fonts (falls back to the system monospace offline — flash
// stays tiny, no fonts embedded), neon cyan/magenta on near-black, scanlines +
// grain + vignette + gridline overlays, HUD corner brackets, and a code-window
// frame for the config form. The heavy animated chip SVG / PCB traces from the
// mock are intentionally omitted; live status widgets take that space instead.
const PAGE_HEAD: &str = "<!DOCTYPE html><html lang='en'><head><meta charset='utf-8'>\
<meta name='viewport' content='width=device-width,initial-scale=1'><title>JZF407 // SIGNAL.EDGE</title>\
<link rel='preconnect' href='https://fonts.gstatic.com' crossorigin>\
<link href='https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;700;800&display=swap' rel='stylesheet'>\
<style>\
:root{--bg:#05060a;--bg2:#090b14;--ink:#e8f6ff;--dim:#6a7891;--dimmer:#3a445a;--line:#18253b;--line-hot:#1f3a6b;--cy:#00e7ff;--cys:#6df1ff;--mg:#ff2bd6;--mg2:#ff6ae3;--vi:#7a4cff;--am:#ffb648;--gr:#36ffb2;--grid:72px}\
*{box-sizing:border-box;margin:0;padding:0}\
html,body{background:var(--bg);color:var(--ink);font-family:'JetBrains Mono',ui-monospace,Menlo,Consolas,monospace;font-weight:400;-webkit-font-smoothing:antialiased}\
body{min-height:100vh;overflow-x:hidden;background:radial-gradient(1200px 700px at 78% 18%,rgba(255,43,214,.10),transparent 65%),radial-gradient(900px 700px at 8% 80%,rgba(0,231,255,.10),transparent 60%),radial-gradient(1400px 900px at 50% 110%,rgba(122,76,255,.07),transparent 60%),var(--bg)}\
.scanlines,.grain,.vignette,.gridlines{position:fixed;inset:0;pointer-events:none;z-index:50}\
.scanlines{background:repeating-linear-gradient(to bottom,rgba(255,255,255,.025) 0 1px,transparent 1px 3px);mix-blend-mode:overlay;opacity:.6}\
.grain{background-image:url(\"data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='160' height='160'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='2' stitchTiles='stitch'/><feColorMatrix values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 0.35 0'/></filter><rect width='100%25' height='100%25' filter='url(%23n)' opacity='0.5'/></svg>\");opacity:.08;mix-blend-mode:overlay}\
.vignette{background:radial-gradient(120% 80% at 50% 50%,transparent 55%,rgba(0,0,0,.65) 100%)}\
.gridlines{background-image:linear-gradient(to right,rgba(24,37,59,.55) 1px,transparent 1px),linear-gradient(to bottom,rgba(24,37,59,.55) 1px,transparent 1px);background-size:var(--grid) var(--grid);-webkit-mask-image:radial-gradient(120% 80% at 50% 40%,#000 30%,transparent 75%);mask-image:radial-gradient(120% 80% at 50% 40%,#000 30%,transparent 75%);opacity:.5;z-index:1}\
.topbar{position:relative;z-index:10;display:grid;grid-template-columns:1fr auto 1fr;align-items:center;gap:24px;padding:14px 28px;border-bottom:1px solid var(--line);font-size:11px;letter-spacing:.18em;text-transform:uppercase;color:var(--dim);background:linear-gradient(to bottom,rgba(0,0,0,.6),transparent);backdrop-filter:blur(2px)}\
.topbar .left,.topbar .right{display:flex;gap:22px;align-items:center}\
.topbar .right{justify-content:flex-end}\
.topbar .brand{color:var(--ink);font-weight:700;letter-spacing:.3em;display:flex;align-items:center;gap:10px}\
.topbar .brand::before{content:'';width:8px;height:8px;background:var(--cy);box-shadow:0 0 12px var(--cy),0 0 24px var(--cy);border-radius:50%;animation:pulse 1.6s ease-in-out infinite}\
@keyframes pulse{0%,100%{opacity:1;transform:scale(1)}50%{opacity:.45;transform:scale(.8)}}\
.topbar .ok{color:var(--gr)}.topbar .hot{color:var(--mg)}\
.tdot{width:5px;height:5px;background:var(--dimmer);border-radius:50%;display:inline-block;margin:0 2px 1px}\
main{position:relative;z-index:5;max-width:1180px;margin:0 auto;padding:44px 28px 60px}\
.eyebrow{display:inline-flex;align-items:center;gap:10px;font-size:11px;letter-spacing:.32em;text-transform:uppercase;color:var(--mg);padding:6px 10px;border:1px solid rgba(255,43,214,.45);background:linear-gradient(180deg,rgba(255,43,214,.08),rgba(255,43,214,0));margin-bottom:24px}\
.eyebrow::before{content:'';width:6px;height:6px;background:var(--mg);box-shadow:0 0 10px var(--mg)}\
h1.title{font-weight:800;font-size:clamp(48px,8vw,108px);line-height:.86;letter-spacing:-.04em;margin:0 0 16px;color:var(--ink);text-shadow:0 0 24px rgba(0,231,255,.18),0 0 80px rgba(0,231,255,.08)}\
h1.title .accent{color:transparent;-webkit-text-stroke:1.5px var(--cy);text-shadow:0 0 24px rgba(0,231,255,.45)}\
h1.title .slash{color:var(--mg);text-shadow:0 0 22px rgba(255,43,214,.6)}\
h1.title::after{content:'_';color:var(--cy);margin-left:-.05em;animation:blink 1.05s steps(2,end) infinite;text-shadow:0 0 22px rgba(0,231,255,.7)}\
@keyframes blink{0%,55%{opacity:1}55.01%,100%{opacity:0}}\
.lede{max-width:620px;font-size:13px;line-height:1.7;color:var(--dim);letter-spacing:.02em;margin:0 0 30px}\
.lede b{color:var(--ink);font-weight:500}.lede .pink{color:var(--mg2);font-weight:500}\
.specs{display:grid;grid-template-columns:repeat(4,1fr);border-top:1px solid var(--line);border-bottom:1px solid var(--line);margin-bottom:34px}\
.spec{padding:18px 18px 16px 0;border-right:1px solid var(--line);position:relative}\
.spec:last-child{border-right:none}\
.spec .k{font-size:10px;letter-spacing:.24em;color:var(--dim);text-transform:uppercase;margin-bottom:10px;display:flex;align-items:center;gap:6px}\
.spec .k::before{content:'//';color:var(--dimmer)}\
.spec .v{font-size:26px;font-weight:700;letter-spacing:-.02em;color:var(--ink);line-height:1;font-variant-numeric:tabular-nums}\
.spec .u{font-size:13px;color:var(--cy);margin-left:4px;font-weight:400}\
.spec .sub{font-size:10px;color:var(--dimmer);letter-spacing:.2em;text-transform:uppercase;margin-top:8px}\
.grid{display:grid;grid-template-columns:1fr 1fr;gap:48px;align-items:start}\
@media(max-width:980px){.grid{grid-template-columns:1fr;gap:36px}.topbar{grid-template-columns:1fr auto}.topbar .right{display:none}}\
.bracketed{position:relative}\
.bracketed::before,.bracketed::after,.bracketed>.bk1,.bracketed>.bk2{content:'';position:absolute;width:14px;height:14px;border:1px solid var(--cy);opacity:.7}\
.bracketed::before{top:-1px;left:-1px;border-right:none;border-bottom:none}\
.bracketed::after{top:-1px;right:-1px;border-left:none;border-bottom:none}\
.bracketed>.bk1{bottom:-1px;left:-1px;border-right:none;border-top:none}\
.bracketed>.bk2{bottom:-1px;right:-1px;border-left:none;border-top:none}\
.section-num{font-size:11px;letter-spacing:.34em;color:var(--dim);text-transform:uppercase;margin-bottom:18px;display:flex;align-items:center;gap:12px}\
.section-num .bar{flex:1;height:1px;background:linear-gradient(to right,var(--line-hot),transparent)}\
.section-num .mg{color:var(--mg)}\
.win{position:relative;border:1px solid var(--line-hot);background:linear-gradient(180deg,rgba(0,231,255,.04),rgba(0,0,0,0) 40%),var(--bg2);box-shadow:0 0 0 1px rgba(0,231,255,.06),0 30px 80px rgba(0,0,0,.55),inset 0 0 60px rgba(0,231,255,.03);margin-bottom:34px}\
.win::before,.win::after,.win>.bk1,.win>.bk2{content:'';position:absolute;width:10px;height:10px;border:1px solid var(--cy)}\
.win::before{top:-1px;left:-1px;border-right:none;border-bottom:none}\
.win::after{top:-1px;right:-1px;border-left:none;border-bottom:none}\
.win>.bk1{bottom:-1px;left:-1px;border-right:none;border-top:none}\
.win>.bk2{bottom:-1px;right:-1px;border-left:none;border-top:none}\
.chrome{display:flex;align-items:center;justify-content:space-between;padding:12px 18px;border-bottom:1px solid var(--line);font-size:11px;letter-spacing:.24em;text-transform:uppercase;color:var(--dim);background:rgba(0,0,0,.35)}\
.chrome .path{color:var(--ink)}.chrome .path .slash{color:var(--mg)}\
.lights{display:inline-flex;gap:6px;margin-right:14px}\
.lights i{display:inline-block;width:9px;height:9px;border:1px solid var(--line-hot)}\
.lights i:nth-child(1){background:rgba(255,43,214,.55);border-color:rgba(255,43,214,.7);box-shadow:0 0 8px rgba(255,43,214,.6)}\
.lights i:nth-child(2){background:rgba(255,182,72,.55);border-color:rgba(255,182,72,.7)}\
.lights i:nth-child(3){background:rgba(54,255,178,.55);border-color:rgba(54,255,178,.7);box-shadow:0 0 8px rgba(54,255,178,.4)}\
.wbody{padding:20px 22px}\
.io{display:flex;align-items:center;justify-content:space-between;gap:14px;padding:14px 0;border-bottom:1px solid var(--line)}\
.io:last-child{border-bottom:none}\
.io .lbl{font-size:13px;letter-spacing:.04em}.io .lbl small{display:block;font-size:10px;letter-spacing:.2em;text-transform:uppercase;color:var(--dimmer);margin-top:3px}\
.pill{display:inline-flex;align-items:center;gap:8px;font-size:11px;font-weight:700;letter-spacing:.18em;text-transform:uppercase;padding:6px 12px;border:1px solid var(--line-hot);background:rgba(0,231,255,.05)}\
.pill.on{color:var(--gr);border-color:rgba(54,255,178,.5);background:rgba(54,255,178,.07);box-shadow:0 0 14px rgba(54,255,178,.18),inset 0 0 12px rgba(54,255,178,.08)}\
.pill.off{color:var(--dim);border-color:var(--line-hot);background:rgba(0,0,0,.25)}\
.dot{width:7px;height:7px;border-radius:50%;background:currentColor;box-shadow:0 0 8px currentColor}\
.kv{display:flex;justify-content:space-between;align-items:center;padding:11px 0;border-bottom:1px solid var(--line);font-size:12px}\
.kv:last-child{border-bottom:none}\
.kv .k{color:var(--dim);letter-spacing:.14em;text-transform:uppercase;font-size:11px}\
.kv .v{font-weight:500;font-variant-numeric:tabular-nums;color:var(--ink)}\
.sec{font-size:10px;font-weight:700;color:var(--cy);text-transform:uppercase;letter-spacing:.24em;margin:22px 0 10px;display:flex;align-items:center;gap:10px}\
.sec::after{content:'';flex:1;height:1px;background:linear-gradient(to right,var(--line-hot),transparent)}\
.sec.first{margin-top:0}\
label{display:block;font-size:10px;font-weight:600;color:var(--dim);letter-spacing:.18em;text-transform:uppercase;margin:12px 0 5px}\
input[type=text],input[type=number],input[type=password]{width:100%;font:inherit;font-size:14px;padding:10px 12px;border:1px solid var(--line-hot);background:rgba(0,0,0,.35);color:var(--ink)}\
input:focus{outline:0;border-color:var(--cy);background:rgba(0,231,255,.04);box-shadow:0 0 0 1px var(--cy),inset 0 0 18px rgba(0,231,255,.06)}\
.row{display:flex;gap:12px}.row>div{flex:1}\
.btn{appearance:none;cursor:pointer;font:inherit;font-size:12px;font-weight:600;letter-spacing:.24em;text-transform:uppercase;padding:14px 22px;background:transparent;color:var(--cy);border:1px solid var(--cy);position:relative;transition:all .2s ease;display:inline-flex;align-items:center;justify-content:center;gap:10px;width:100%}\
.btn::before,.btn::after{content:'';position:absolute;width:8px;height:8px;border:1px solid currentColor}\
.btn::before{top:-1px;left:-1px;border-right:none;border-bottom:none}\
.btn::after{bottom:-1px;right:-1px;border-left:none;border-top:none}\
.btn:hover{background:rgba(0,231,255,.12);box-shadow:0 0 20px rgba(0,231,255,.3),inset 0 0 18px rgba(0,231,255,.1)}\
.btn.save{color:#fff;border-color:var(--mg);background:linear-gradient(180deg,rgba(255,43,214,.16),rgba(255,43,214,.04));box-shadow:0 0 22px rgba(255,43,214,.25),inset 0 0 30px rgba(255,43,214,.12);margin-top:24px}\
.btn.save:hover{background:rgba(255,43,214,.22);box-shadow:0 0 26px rgba(255,43,214,.4)}\
.btn.reboot{color:var(--mg);border-color:rgba(255,43,214,.5)}\
.btn.reboot:hover{background:rgba(255,43,214,.1);box-shadow:0 0 20px rgba(255,43,214,.3)}\
.foot{padding:16px 18px;border-top:1px solid var(--line);display:flex;gap:12px;background:rgba(0,0,0,.35)}\
@media(max-width:520px){.foot{flex-direction:column}.specs{grid-template-columns:repeat(2,1fr)}.spec:nth-child(2){border-right:none}}\
.toast{position:fixed;bottom:24px;left:50%;transform:translateX(-50%);background:var(--bg2);border:1px solid var(--mg);color:var(--ink);padding:12px 24px;font-size:11px;font-weight:600;letter-spacing:.16em;text-transform:uppercase;z-index:999;opacity:0;transition:opacity .3s;box-shadow:0 0 24px rgba(255,43,214,.3)}\
.toast.show{opacity:1}\
</style></head><body>\
<div class='gridlines'></div><div class='scanlines'></div><div class='grain'></div><div class='vignette'></div>";

// Footer script. Three concerns:
// - Live updates: SSE (EventSource /events) pushes relay/led/link/mqtt the
//   instant they change. A /state poll runs only as a FALLBACK while SSE is
//   disconnected (old browser, proxy that buffers event-streams, dropped link),
//   so a healthy page makes ~0 polling traffic.
// - Live clock/uptime: ticked locally every second from the browser's Date()
//   and a base uptime re-synced on each SSE/poll payload (the MCU has no RTC).
// - Auto-logout: 15-min inactivity → POST /logout (server session is the real
//   gate; this is the convenience timer with a 60 s toast warning).
const PAGE_FOOT: &str = "<script>\
function g(i){return document.getElementById(i)}\
function pl(i,o,t){var e=g(i);if(e){e.className='pill '+(o?'on':'off');e.innerHTML=\"<span class='dot'></span>\"+t}}\
function tx(i,v){var e=g(i);if(e){e.textContent=v}}\
function z(n){return(n<10?'0':'')+n}\
function fu(n){var d=n/86400|0,h=n%86400/3600|0,m=n%3600/60|0,s=n%60;return (d?d+'d ':'')+z(h)+':'+z(m)+':'+z(s)}\
function clock(){var n=new Date();tx('clk',z(n.getHours())+':'+z(n.getMinutes())+':'+z(n.getSeconds()))}\
var loggingOut=false;\
function toLogin(){if(loggingOut)return;loggingOut=true;location.href='/login'}\
var IDLE_TIMEOUT=900,lastActivity=Date.now();\
var upBase=0,upAt=0;\
function apply(s){pl('rp',s.relay,s.relay?'ON':'OFF');pl('l1',s.led1,'LED1');pl('l2',s.led2,'LED2');pl('lk',s.link,s.link?'Up':'Down');pl('mq',s.mqtt,s.mqtt?'Online':'Offline');if(s.ip!==undefined)tx('ip',s.ip);if(s.up!==undefined){upBase=s.up;upAt=Date.now()}}\
function resetIdle(){lastActivity=Date.now();if(g('toast'))g('toast').className='toast'}\
['mousemove','keydown','touchstart','scroll','click'].forEach(function(e){document.addEventListener(e,resetIdle,{passive:true})});\
function doLogout(){if(loggingOut)return;loggingOut=true;var f=document.createElement('form');f.method='post';f.action='/logout';document.body.appendChild(f);f.submit()}\
var sseUp=false,es=null;\
function sse(){if(loggingOut||!window.EventSource)return;try{es=new EventSource('/events')}catch(e){return}\
es.onopen=function(){sseUp=true};\
es.onmessage=function(m){try{apply(JSON.parse(m.data))}catch(e){}};\
es.onerror=function(){sseUp=false;try{es.close()}catch(e){}es=null;setTimeout(sse,3000)}}\
function poll(){if(loggingOut||sseUp)return;var a=(Date.now()-lastActivity)/1000<30?'?active=1':'';fetch('/state'+a,{credentials:'same-origin'}).then(function(r){if(r.status===401){toLogin();return null}return r.ok?r.json():null}).then(function(s){if(s)apply(s)}).catch(function(){})}\
setInterval(function(){\
clock();\
if(upAt)tx('up',fu(upBase+Math.floor((Date.now()-upAt)/1000)));\
if(loggingOut)return;\
poll();\
var idle=(Date.now()-lastActivity)/1000;\
if(idle>=IDLE_TIMEOUT){doLogout()}\
else if(idle>=IDLE_TIMEOUT-60&&g('toast')){g('toast').className='toast show';g('toast').textContent='Session expires in '+(IDLE_TIMEOUT-Math.floor(idle))+'s — move mouse to stay'}\
},1000);\
clock();sse();poll();\
</script><div id='toast' class='toast'></div></body></html>";

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

/// Stream the dashboard page directly to the socket. Streaming flash-resident
/// chunks avoids building a multi-KB HTML buffer in RAM. Layout mirrors the
/// "SIGNAL.EDGE" mock: topbar → hero (eyebrow/title/lede/specs) → 2-column grid
/// (left = live I/O + telemetry windows, right = config code-window) → footer.
/// All numbers are this board's real values (F407 @168 MHz, 512K/128K), not the
/// mock's fictional M7. Pill/text ids (rp/l1/l2/lk/mq/ip/up) are the hooks the
/// SSE/poll script repaints live.
async fn send_page(
    socket: &mut TcpSocket<'_>,
    cfg: &NetworkConfig,
    stack: Stack<'static>,
    reset: ResetReason,
) {
    let relay_on = crate::OUTPUTS.get_relay();
    let led1_on = crate::OUTPUTS.get_led(crate::LedId::Led1);
    let led2_on = crate::OUTPUTS.get_led(crate::LedId::Led2);
    let link_up = stack.is_link_up();
    let mqtt_up = crate::mqtt::MQTT_ONLINE.load(Ordering::Relaxed);
    let ip = fmt_ipv4(&cfg.ip);
    let gw = fmt_ipv4(&cfg.gateway);
    let bk = fmt_ipv4(&cfg.broker_ip);
    let port = fmt_u16(cfg.broker_port);
    let prefix = fmt_u8(cfg.prefix_len);

    w(socket, HTTP_OK).await;
    w(socket, PAGE_HEAD).await;

    // ---- Top status bar. Brand + section coords on the left; live LINK/MQTT
    // pills + browser clock on the right (clock filled by JS from Date()). ----
    w(socket, "<div class='topbar'><div class='left'><span class='brand'>JZF407 // SIGNAL.EDGE</span><span>SEC \u{25b8} STM32F407VET6</span></div>\
<div class='brand' style='font-size:10px;letter-spacing:.4em'>\u{2014} REV 2.2 \u{2014}</div>\
<div class='right'><span>UPLINK ").await;
    w(socket, if link_up { "<span class='ok'>\u{25cf}</span> NOMINAL" } else { "<span class='hot'>\u{25cf}</span> DOWN" }).await;
    w(socket, "</span><span>BUS <span class='hot'>\u{25cf}</span> 168MHZ</span><span id='clk'>--:--:--</span></div></div>").await;

    // ---- Hero: eyebrow + glitch title + lede + real-spec grid ----
    w(socket, "<main><span class='eyebrow'>Arm\u{00ae} Cortex\u{00ae}-M4F // Embassy async firmware</span>\
<h1 class='title'>JZF<span class='slash'>/</span><span class='accent'>407</span></h1>\
<p class='lede'>STM32F407VET6 on the Embassy async executor \u{2014} <b>real-time GPIO, Ethernet, and MQTT</b> over a smoltcp stack. Relay drives a <span class='pink'>2 s monostable pulse</span>, LEDs persist to EEPROM, and this page streams live state over <b>Server-Sent Events</b> \u{2014} no polling, instant updates.</p>").await;

    // Spec grid — this board's actual silicon, not the mock's numbers.
    // uptime cell (id=up) is filled live by the script.
    w(socket, "<div class='specs bracketed'><span class='bk1'></span><span class='bk2'></span>\
<div class='spec'><div class='k'>core</div><div class='v'>M4F<span class='u'>@168</span></div><div class='sub'>Cortex-M4 + FPU</div></div>\
<div class='spec'><div class='k'>flash</div><div class='v'>512<span class='u'>KB</span></div><div class='sub'>~28% used</div></div>\
<div class='spec'><div class='k'>sram</div><div class='v'>128<span class='u'>KB</span></div><div class='sub'>+64K CCM</div></div>\
<div class='spec'><div class='k'>uptime</div><div class='v' id='up' style='font-size:20px'>\u{2014}</div><div class='sub' id='rst'>").await;
    w(socket, reset.as_str()).await;
    w(socket, "</div></div></div>").await;

    // ---- 2-column grid: left = I/O + telemetry, right = config ----
    w(socket, "<div class='grid'><div>").await;

    // I/O window (relay + LEDs). Relay is MQTT-only (stm32/relay) → 2 s pulse.
    w(socket, "<div class='section-num'><span>0X01</span><span class='bar'></span><span class='mg'>// I/O STATE</span></div>\
<div class='win'><span class='bk1'></span><span class='bk2'></span>\
<div class='chrome'><span class='path'>gpio<span class='slash'>/</span>outputs</span><span class='lights'><i></i><i></i><i></i></span></div>\
<div class='wbody'>\
<div class='io'><div class='lbl'>RELAY<small>MQTT stm32/relay \u{2192} 2s pulse</small></div>").await;
    pill(socket, "rp", relay_on, if relay_on { "ON" } else { "OFF" }).await;
    w(socket, "</div><div class='io'><div class='lbl'>LED 1<small>stm32/led/1 \u{00b7} persisted</small></div>").await;
    pill(socket, "l1", led1_on, "LED1").await;
    w(socket, "</div><div class='io'><div class='lbl'>LED 2<small>stm32/led/2 \u{00b7} persisted</small></div>").await;
    pill(socket, "l2", led2_on, "LED2").await;
    w(socket, "</div></div></div>").await;

    // Telemetry window. lk/mq pills + ip text repaint live.
    w(socket, "<div class='section-num'><span>0X02</span><span class='bar'></span><span class='mg'>// LINK</span></div>\
<div class='win'><span class='bk1'></span><span class='bk2'></span>\
<div class='chrome'><span class='path'>net<span class='slash'>/</span>status</span><span class='lights'><i></i><i></i><i></i></span></div>\
<div class='wbody'>\
<div class='kv'><span class='k'>Ethernet</span>").await;
    pill(socket, "lk", link_up, if link_up { "Up" } else { "Down" }).await;
    w(socket, "</div><div class='kv'><span class='k'>MQTT Broker</span>").await;
    pill(socket, "mq", mqtt_up, if mqtt_up { "Online" } else { "Offline" }).await;
    w(socket, "</div><div class='kv'><span class='k'>IP Address</span><span class='v' id='ip'>").await;
    w(socket, ip.as_str()).await;
    w(socket, "</span></div></div></div></div>").await;

    // ---- Right column: config code-window ----
    w(socket, "<div><div class='section-num'><span>0X03</span><span class='bar'></span><span class='mg'>// CONFIG</span></div>\
<div class='win'><span class='bk1'></span><span class='bk2'></span>\
<div class='chrome'><span class='path'>etc<span class='slash'>/</span>config.toml</span><span>EEPROM</span></div>\
<div class='wbody'><form method='post' action='/save'>\
<div class='sec first'>Network</div><label>IP address</label><input type='text' name='ip' value='").await;
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

    // MQTT broker credentials (sent in the CONNECT packet). Empty = anonymous.
    w(socket, "'><div class='sec'>MQTT Auth</div><div class='row'><div><label>Username</label><input type='text' name='muser' value='").await;
    w(socket, cfg.mqtt_user.as_str()).await;
    w(socket, "'></div><div><label>Password</label><input type='password' name='mpass' value='").await;
    w(socket, cfg.mqtt_pass.as_str()).await;

    // Web login (form + session cookie). Leaving BOTH blank disables the login.
    w(socket, "'></div></div><div class='sec'>Web Login</div><div class='row'><div><label>Username</label><input type='text' name='wuser' value='").await;
    w(socket, cfg.web_user.as_str()).await;
    w(socket, "'></div><div><label>Password</label><input type='password' name='wpass' value='").await;
    w(socket, cfg.web_pass.as_str()).await;

    // Close form inputs, Save button (inside form), then footer with Reboot +
    // Logout. Close .wbody / .win / right grid col / .grid / main.
    w(socket, "'></div></div><button class='btn save'>\u{25b8} Save &amp; Reboot</button></form></div>\
<div class='foot'><form method='post' action='/reboot' style='flex:1'><button class='btn reboot'>Reboot</button></form>\
<button class='btn' style='flex:1' onclick='doLogout()'>Log Out</button></div>\
</div></div></div></main>").await;

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

/// Two-socket web pool. Both instances run the identical router (`serve_connection`),
/// so either socket can serve a normal request OR the long-lived `GET /events`
/// SSE stream — a request never lands on a socket that can't handle it. One
/// socket can be parked in an SSE stream while the other keeps serving the page,
/// /state, /save, etc. Sockets come from the `StackResources<8>` pool in main.
///
/// Each instance needs its OWN buffers; embassy tasks can't share `StaticCell`s,
/// hence two near-identical task fns rather than a spawned-N loop.
#[embassy_executor::task]
pub async fn web_task(stack: Stack<'static>, cfg: NetworkConfig, reset_reason: ResetReason) {
    info!("WEB: waiting for link...");
    stack.wait_link_up().await;
    info!("WEB: link up");
    stack.wait_config_up().await;
    info!("WEB: listening on :80 (pool A)");

    static RX_BUF: static_cell::StaticCell<[u8; 1536]> = static_cell::StaticCell::new();
    static TX_BUF: static_cell::StaticCell<[u8; 4096]> = static_cell::StaticCell::new();
    static REQ_BUF: static_cell::StaticCell<[u8; 1536]> = static_cell::StaticCell::new();
    let rx_buf = RX_BUF.init([0u8; 1536]);
    let tx_buf = TX_BUF.init([0u8; 4096]);
    let req_buf = REQ_BUF.init([0u8; 1536]);
    serve_pool(stack, cfg, reset_reason, rx_buf, tx_buf, req_buf).await;
}

/// Second pool instance — see [`web_task`]. Exists only to own a distinct set of
/// `StaticCell` buffers.
#[embassy_executor::task]
pub async fn web_task_b(stack: Stack<'static>, cfg: NetworkConfig, reset_reason: ResetReason) {
    stack.wait_link_up().await;
    stack.wait_config_up().await;
    info!("WEB: listening on :80 (pool B)");

    static RX_BUF: static_cell::StaticCell<[u8; 1536]> = static_cell::StaticCell::new();
    static TX_BUF: static_cell::StaticCell<[u8; 4096]> = static_cell::StaticCell::new();
    static REQ_BUF: static_cell::StaticCell<[u8; 1536]> = static_cell::StaticCell::new();
    let rx_buf = RX_BUF.init([0u8; 1536]);
    let tx_buf = TX_BUF.init([0u8; 4096]);
    let req_buf = REQ_BUF.init([0u8; 1536]);
    serve_pool(stack, cfg, reset_reason, rx_buf, tx_buf, req_buf).await;
}

/// The accept loop shared by both pool instances. Holds the per-instance buffers
/// and the auth setup; delegates each accepted connection to `serve_connection`.
async fn serve_pool(
    stack: Stack<'static>,
    cfg: NetworkConfig,
    reset_reason: ResetReason,
    rx_buf: &mut [u8],
    tx_buf: &mut [u8],
    req_buf: &mut [u8],
) {
    // Form-login + session cookie (no HTTP Basic Auth — browsers cache and
    // auto-resend Basic credentials, making logout impossible). Auth is enabled
    // only when a web username or password is set; an all-empty pair leaves the
    // page open (matches a fresh/upgraded device on a trusted LAN). The expected
    // token is derived from the credentials once here and compared against what
    // POST /login submits; on a match we mint the session cookie.
    let auth_required = !cfg.web_user.is_empty() || !cfg.web_pass.is_empty();
    let expected_token =
        jzf407_logic::auth::basic_token(cfg.web_user.as_str(), cfg.web_pass.as_str());
    if auth_required {
        info!("WEB: form-login auth enabled");
    } else {
        warn!("WEB: no web password set — page is open to anyone who can reach it");
    }

    // Seed the token generator with the current uptime (monotonic, no RTC needed).
    let token_seed = Instant::now().as_secs().wrapping_mul(0x9E3779B97F4A7C15);

    loop {
        let mut socket = TcpSocket::new(stack, &mut *rx_buf, &mut *tx_buf);
        socket.set_timeout(Some(Duration::from_secs(10)));
        // Disable Nagle: with ~30 small write_all() calls per page, Nagle + the
        // host's delayed-ACK (40–200 ms) would stall each write until the previous
        // segment is ACKed, making the page take several seconds to load.
        socket.set_nagle_enabled(false);

        if socket.accept(80).await.is_err() {
            Timer::after(Duration::from_millis(100)).await;
            continue;
        }

        serve_connection(
            &mut socket,
            &cfg,
            stack,
            reset_reason,
            req_buf,
            auth_required,
            &expected_token,
            token_seed,
        )
        .await;
    }
}

/// Handle one accepted connection: read the request, run the auth gate, route to
/// the matching handler (incl. the `/events` SSE stream), and close. Lives apart
/// from the accept loop so both pool instances share exactly one router.
#[allow(clippy::too_many_arguments)]
async fn serve_connection(
    socket: &mut TcpSocket<'_>,
    cfg: &NetworkConfig,
    stack: Stack<'static>,
    reset_reason: ResetReason,
    req_buf: &mut [u8],
    auth_required: bool,
    expected_token: &str,
    token_seed: u64,
) {
        let n = match socket.read(req_buf).await {
            Ok(n) if n > 0 => n,
            _ => return,
        };

        let req = match core::str::from_utf8(&req_buf[..n]) {
            Ok(s) => s,
            Err(_) => {
                let _ = socket.write_all(HTTP_400.as_bytes()).await;
                return;
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
        // and the long-lived /events stream refresh it ONLY when the page reports
        // recent user activity (?active=1 on /state) — so an open-but-abandoned
        // tab can't keep the session alive by polling or by holding /events open,
        // but an actively-used page stays logged in.
        let refresh_ttl = !(path == "/state" || path == "/events") || query.contains("active=1");
        let authenticated = !auth_required || validate_session(req, refresh_ttl);

        if !authenticated {
            // POST /login — validate form-submitted credentials (user & pass).
            if method == "POST" && path == "/login" {
                let login_ok = if let Some(body_start) = req.find("\r\n\r\n") {
                    let body = &req[body_start + 4..];
                    let (user_opt, pass_opt) = parse_login_form(body);
                    if let (Some(u), Some(p)) = (user_opt, pass_opt) {
                        let submitted = jzf407_logic::auth::basic_token(u.as_str(), p.as_str());
                        submitted.as_str() == expected_token
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
                    w(socket, redirect.as_str()).await;
                    let _ = socket.flush().await;
                    socket.close();
                    return;
                }
                // Credentials wrong — re-show the login form with an error.
                w(socket, HTTP_401_LOGIN).await;
                w(socket, LOGIN_HEAD).await;
                w(socket, LOGIN_ERROR).await;
                w(socket, LOGIN_TAIL).await;
                let _ = socket.flush().await;
                socket.close();
                return;
            }

            // Unauthenticated /events: 401 + close so the EventSource errors out
            // and the page falls back to /state polling (which 401s → /login).
            // No login form body on this path — it isn't a navigable page.
            if path == "/events" {
                w(socket, HTTP_401_LOGIN).await;
                let _ = socket.flush().await;
                socket.close();
                return;
            }

            // Not authenticated and not a login POST: show the login form.
            // If a (now-invalid/expired) session cookie is present, expire it in
            // the same response so the browser drops it. No WWW-Authenticate
            // header anywhere — the browser must never pop its native Basic dialog.
            if extract_cookie(req, "jzf_session").is_some() {
                w(socket, HTTP_401_EXPIRE_COOKIE).await;
            } else {
                w(socket, HTTP_401_LOGIN).await;
            }
            w(socket, LOGIN_HEAD).await;
            w(socket, LOGIN_TAIL).await;
            let _ = socket.flush().await;
            socket.close();
            return;
        }

        match (method, path) {
            ("GET", "/") => {
                // The session cookie is minted only by POST /login. Here we just
                // render the page; the request already carried a valid cookie
                // (auth gate passed), so no Set-Cookie is needed.
                send_page(socket, cfg, stack, reset_reason).await;
                // Graceful FIN so the browser gets a clean EOF, not a RST.
                // Without this, drop() calls abort() → RST → fetch('/state')
                // fails with ERR_CONNECTION_RESET and live updates stop working.
                socket.close();
            }
            // Live push stream. Holds THIS socket open for the life of the page;
            // the other pool instance keeps serving everything else. Authenticated
            // by the gate above (same session as the rest of the site).
            ("GET", "/events") => {
                crate::sse::serve_events(socket, stack).await;
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
                send_state(socket, cfg, stack, reset_reason).await;
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
                    match parse_form(body, cfg) {
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
