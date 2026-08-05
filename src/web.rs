use crate::fault_marker::ResetReason;
use core::sync::atomic::{AtomicU32, Ordering};
use defmt::{info, warn};
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::Write;
use heapless::String as HString;
use jzf407_logic::config::{parse_ipv4, parse_port, NetworkConfig};
use jzf407_logic::web_form::{form_url_decode, html_escape_attr, secret_from_form};

const HTTP_OK: &str =
    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n";
const HTTP_OK_JSON: &str =
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n";
const HTTP_303_HOME: &str = "HTTP/1.1 303 See Other\r\nLocation: /\r\nConnection: close\r\n\r\n";

// Cookie clearing needs Max-Age=0 + past Expires and the same Path=/ it was set
// with, or the browser keeps it.
const HTTP_303_LOGOUT: &str = "HTTP/1.1 303 See Other\r\n\
Set-Cookie: jzf_session=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; SameSite=Strict\r\n\
Location: /login\r\nConnection: close\r\n\r\n";
const HTTP_400: &str = "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nBad Request";
const HTTP_404: &str = "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\nNot Found";
const HTTP_500: &str = "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<html><body><p>EEPROM write failed \u{2014} config NOT saved. <a href=/>back</a></p></body></html>";

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

const LOGIN_ERROR: &str = "<div class='err'>\u{2715} Invalid username or password</div>";
const LOGIN_LOCKED: &str =
    "<div class='err'>\u{26a0} Too many failed attempts \u{2014} locked for 60s</div>";

const LOGIN_TAIL: &str = "<form method='post' action='/login'>\
<label>Username</label><input type='text' name='user' placeholder='user' autocomplete='username' required>\
<label>Password</label><input type='password' name='pass' placeholder='\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}' autocomplete='current-password' required>\
<button type='submit'>\u{25b8} Authenticate</button>\
</form></div></div></body></html>";

// No WWW-Authenticate header anywhere: it would pop the browser's native Basic
// Auth dialog instead of our form, and cached Basic credentials make logout impossible.
const HTTP_401_LOGIN: &str = "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n";

const HTTP_401_EXPIRE_COOKIE: &str = "HTTP/1.1 401 Unauthorized\r\n\
Set-Cookie: jzf_session=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; SameSite=Strict\r\n\
Content-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n";

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

// Footer script: SSE live updates with /state polling only as fallback,
// locally-ticked clock/uptime (no RTC), 15-min idle auto-logout (server session
// is the real gate).
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

async fn write_attr(socket: &mut TcpSocket<'_>, s: &str) {
    if let Ok(escaped) = html_escape_attr::<256>(s) {
        w(socket, escaped.as_str()).await;
    }
}

async fn pill(socket: &mut TcpSocket<'_>, id: &str, on: bool, text: &str) {
    w(socket, "<span class='pill ").await;
    w(socket, if on { "on" } else { "off" }).await;
    w(socket, "' id='").await;
    w(socket, id).await;
    w(socket, "'><span class='dot'></span>").await;
    w(socket, text).await;
    w(socket, "</span>").await;
}

// Hit ~once per second per open page — intentionally logs nothing.
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

// Streamed in chunks to avoid a multi-KB page buffer in RAM.
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

    w(socket, "<div class='topbar'><div class='left'><span class='brand'>JZF407 // SIGNAL.EDGE</span><span>SEC \u{25b8} STM32F407VET6</span></div>\
<div class='brand' style='font-size:10px;letter-spacing:.4em'>\u{2014} REV 2.2 \u{2014}</div>\
<div class='right'><span>UPLINK ").await;
    w(
        socket,
        if link_up {
            "<span class='ok'>\u{25cf}</span> NOMINAL"
        } else {
            "<span class='hot'>\u{25cf}</span> DOWN"
        },
    )
    .await;
    w(socket, "</span><span>BUS <span class='hot'>\u{25cf}</span> 168MHZ</span><span id='clk'>--:--:--</span></div></div>").await;

    w(socket, "<main><span class='eyebrow'>Arm\u{00ae} Cortex\u{00ae}-M4F // Embassy async firmware</span>\
<h1 class='title'>JZF<span class='slash'>/</span><span class='accent'>407</span></h1>\
<p class='lede'>STM32F407VET6 on the Embassy async executor \u{2014} <b>real-time GPIO, Ethernet, and MQTT</b> over a smoltcp stack. Relay drives a <span class='pink'>2 s monostable pulse</span>, LEDs persist to EEPROM, and this page streams live state over <b>Server-Sent Events</b> \u{2014} no polling, instant updates.</p>").await;

    w(socket, "<div class='specs bracketed'><span class='bk1'></span><span class='bk2'></span>\
<div class='spec'><div class='k'>core</div><div class='v'>M4F<span class='u'>@168</span></div><div class='sub'>Cortex-M4 + FPU</div></div>\
<div class='spec'><div class='k'>flash</div><div class='v'>512<span class='u'>KB</span></div><div class='sub'>~28% used</div></div>\
<div class='spec'><div class='k'>sram</div><div class='v'>128<span class='u'>KB</span></div><div class='sub'>+64K CCM</div></div>\
<div class='spec'><div class='k'>uptime</div><div class='v' id='up' style='font-size:20px'>\u{2014}</div><div class='sub' id='rst'>").await;
    w(socket, reset.as_str()).await;
    w(socket, "</div></div></div>").await;

    w(socket, "<div class='grid'><div>").await;

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

    w(socket, "<div class='section-num'><span>0X02</span><span class='bar'></span><span class='mg'>// LINK</span></div>\
<div class='win'><span class='bk1'></span><span class='bk2'></span>\
<div class='chrome'><span class='path'>net<span class='slash'>/</span>status</span><span class='lights'><i></i><i></i><i></i></span></div>\
<div class='wbody'>\
<div class='kv'><span class='k'>Ethernet</span>").await;
    pill(socket, "lk", link_up, if link_up { "Up" } else { "Down" }).await;
    w(
        socket,
        "</div><div class='kv'><span class='k'>MQTT Broker</span>",
    )
    .await;
    pill(
        socket,
        "mq",
        mqtt_up,
        if mqtt_up { "Online" } else { "Offline" },
    )
    .await;
    w(
        socket,
        "</div><div class='kv'><span class='k'>IP Address</span><span class='v' id='ip'>",
    )
    .await;
    w(socket, ip.as_str()).await;
    w(socket, "</span></div></div></div></div>").await;

    w(socket, "<div><div class='section-num'><span>0X03</span><span class='bar'></span><span class='mg'>// CONFIG</span></div>\
<div class='win'><span class='bk1'></span><span class='bk2'></span>\
<div class='chrome'><span class='path'>etc<span class='slash'>/</span>config.toml</span><span>EEPROM</span></div>\
<div class='wbody'><form method='post' action='/save'>\
<div class='sec first'>Network</div><label>IP address</label><input type='text' name='ip' value='").await;
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
    write_attr(socket, cfg.client_id.as_str()).await;

    w(socket, "'><div class='sec'>MQTT Auth</div><div class='row'><div><label>Username</label><input type='text' name='muser' value='").await;
    write_attr(socket, cfg.mqtt_user.as_str()).await;
    w(socket, "'></div><div><label>Password</label><input type='password' name='mpass' value='' placeholder='").await;
    w(
        socket,
        if cfg.mqtt_pass.is_empty() {
            ""
        } else {
            "leave blank to keep"
        },
    )
    .await;

    w(socket, "'></div></div><div class='sec'>Web Login</div><div class='row'><div><label>Username</label><input type='text' name='wuser' value='").await;
    write_attr(socket, cfg.web_user.as_str()).await;
    w(socket, "'></div><div><label>Password</label><input type='password' name='wpass' value='' placeholder='").await;
    w(
        socket,
        if cfg.web_pass.is_empty() {
            ""
        } else {
            "leave blank to keep"
        },
    )
    .await;

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

// Single global session (one logged-in client; a new login supersedes the old):
// a 64-bit token split across two atomics + the uptime-second it was refreshed.
static SESSION_TOKEN_HI: AtomicU32 = AtomicU32::new(0);
static SESSION_TOKEN_LO: AtomicU32 = AtomicU32::new(0);
static SESSION_CREATED: AtomicU32 = AtomicU32::new(0);

static LOGIN_FAIL_COUNT: AtomicU32 = AtomicU32::new(0);
static LOGIN_FAIL_LAST: AtomicU32 = AtomicU32::new(0);

const LOGIN_MAX_FAILS: u32 = 5;
const LOGIN_LOCKOUT_SECS: u64 = 60;
const LOGIN_FAIL_DELAY_MS: u64 = 2000;

const SESSION_TTL_SECS: u32 = 900;

fn mint_session_token(seed: u64) -> u64 {
    seed.wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
}

// `refresh=false` (background /state poll) does not slide the idle TTL, so an
// abandoned tab cannot keep the session alive by polling.
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
        SESSION_TOKEN_HI.store(0, Ordering::Relaxed);
        SESSION_TOKEN_LO.store(0, Ordering::Relaxed);
        return false;
    }

    if refresh {
        SESSION_CREATED.store(now as u32, Ordering::Relaxed);
    }
    true
}

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
/// hence each task selects a distinct `WebPoolBuffers` instance.
struct WebPoolBuffers {
    rx: static_cell::StaticCell<[u8; 1536]>,
    tx: static_cell::StaticCell<[u8; 4096]>,
    req: static_cell::StaticCell<[u8; 1536]>,
}

impl WebPoolBuffers {
    const fn new() -> Self {
        Self {
            rx: static_cell::StaticCell::new(),
            tx: static_cell::StaticCell::new(),
            req: static_cell::StaticCell::new(),
        }
    }
}

static WEB_POOL_A: WebPoolBuffers = WebPoolBuffers::new();
static WEB_POOL_B: WebPoolBuffers = WebPoolBuffers::new();

#[embassy_executor::task]
pub async fn web_task(stack: Stack<'static>, cfg: NetworkConfig, reset_reason: ResetReason) {
    info!("WEB: waiting for link...");
    stack.wait_link_up().await;
    info!("WEB: link up");
    stack.wait_config_up().await;
    run_web_pool(stack, cfg, reset_reason, &WEB_POOL_A, "A").await;
}

#[embassy_executor::task]
pub async fn web_task_b(stack: Stack<'static>, cfg: NetworkConfig, reset_reason: ResetReason) {
    stack.wait_link_up().await;
    stack.wait_config_up().await;
    run_web_pool(stack, cfg, reset_reason, &WEB_POOL_B, "B").await;
}

async fn run_web_pool(
    stack: Stack<'static>,
    cfg: NetworkConfig,
    reset_reason: ResetReason,
    buffers: &'static WebPoolBuffers,
    label: &'static str,
) {
    info!("WEB: listening on :80 (pool {})", label);
    let rx_buf = buffers.rx.init([0u8; 1536]);
    let tx_buf = buffers.tx.init([0u8; 4096]);
    let req_buf = buffers.req.init([0u8; 1536]);
    serve_pool(stack, cfg, reset_reason, rx_buf, tx_buf, req_buf).await;
}

async fn serve_pool(
    stack: Stack<'static>,
    cfg: NetworkConfig,
    reset_reason: ResetReason,
    rx_buf: &mut [u8],
    tx_buf: &mut [u8],
    req_buf: &mut [u8],
) {
    let auth_required = !cfg.web_user.is_empty() || !cfg.web_pass.is_empty();
    let expected_token =
        jzf407_logic::auth::basic_token(cfg.web_user.as_str(), cfg.web_pass.as_str());
    if auth_required {
        info!("WEB: form-login auth enabled");
    } else {
        warn!("WEB: no web password set — page is open to anyone who can reach it");
    }

    let token_seed = Instant::now().as_secs().wrapping_mul(0x9E3779B97F4A7C15);

    loop {
        let mut socket = TcpSocket::new(stack, &mut *rx_buf, &mut *tx_buf);
        socket.set_timeout(Some(Duration::from_secs(10)));
        // Nagle + the host's delayed-ACK would stall each of the ~30 small
        // write_all() calls per page, making the page take seconds to load.
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

        // close() only QUEUES the FIN; dropping the socket (or reusing its
        // buffers next iteration) before the flush would abort with RST and
        // break the browser's fetch. The select() bounds a misbehaving peer.
        socket.close();
        let _ = embassy_futures::select::select(
            socket.flush(),
            Timer::after(Duration::from_millis(500)),
        )
        .await;
    }
}

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
        Ok(req) => req,
        Err(_) => {
            let _ = socket.write_all(HTTP_400.as_bytes()).await;
            return;
        }
    };

    let request = ParsedRequest::new(req);
    if !authenticate_request(socket, request, auth_required, expected_token, token_seed).await {
        return;
    }

    route_authenticated(socket, request, cfg, stack, reset_reason).await;
}

#[derive(Clone, Copy)]
struct ParsedRequest<'a> {
    raw: &'a str,
    method: &'a str,
    path: &'a str,
    query: &'a str,
}

impl<'a> ParsedRequest<'a> {
    fn new(raw: &'a str) -> Self {
        let first_line = raw.lines().next().unwrap_or("");
        let mut parts = first_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let target = parts.next().unwrap_or("/");
        let (path, query) = target.split_once('?').unwrap_or((target, ""));

        Self {
            raw,
            method,
            path,
            query,
        }
    }

    fn body(self) -> Option<&'a str> {
        self.raw
            .find("\r\n\r\n")
            .map(|start| &self.raw[start + 4..])
    }

    fn refreshes_session(self) -> bool {
        !(self.path == "/state" || self.path == "/events") || self.query.contains("active=1")
    }
}

/// Form-login + session-cookie only (no HTTP Basic Auth). Browsers cache Basic
/// credentials and auto-resend them, which makes logout impossible. For `/state`
/// and `/events`, refresh the idle TTL only when the page reports recent activity.
async fn authenticate_request(
    socket: &mut TcpSocket<'_>,
    request: ParsedRequest<'_>,
    auth_required: bool,
    expected_token: &str,
    token_seed: u64,
) -> bool {
    if !auth_required {
        return true;
    }
    if validate_session(request.raw, request.refreshes_session()) {
        return true;
    }

    handle_unauthenticated(socket, request, expected_token, token_seed).await;
    false
}

async fn handle_unauthenticated(
    socket: &mut TcpSocket<'_>,
    request: ParsedRequest<'_>,
    expected_token: &str,
    token_seed: u64,
) {
    if request.method == "POST" && request.path == "/login" {
        handle_login(socket, request, expected_token, token_seed).await;
        return;
    }

    // EventSource must receive a bare 401 so it errors and lets polling take over.
    if request.path == "/events" {
        send_and_close(socket, HTTP_401_LOGIN).await;
        return;
    }

    show_login_form(socket, request.raw).await;
}

async fn handle_login(
    socket: &mut TcpSocket<'_>,
    request: ParsedRequest<'_>,
    expected_token: &str,
    token_seed: u64,
) {
    let now = Instant::now().as_secs();
    let fails = LOGIN_FAIL_COUNT.load(Ordering::Relaxed);
    let last_fail = LOGIN_FAIL_LAST.load(Ordering::Relaxed) as u64;

    if login_is_locked(now, fails, last_fail) {
        warn!("WEB: login locked out ({} fails)", fails);
        send_login_form(socket, HTTP_401_LOGIN, Some(LOGIN_LOCKED)).await;
        return;
    }

    if fails >= LOGIN_MAX_FAILS {
        LOGIN_FAIL_COUNT.store(0, Ordering::Relaxed);
    }

    if login_credentials_match(request, expected_token) {
        complete_login(socket, token_seed).await;
        return;
    }

    reject_login(socket).await;
}

fn login_is_locked(now: u64, fails: u32, last_fail: u64) -> bool {
    fails >= LOGIN_MAX_FAILS && now.saturating_sub(last_fail) < LOGIN_LOCKOUT_SECS
}

fn login_credentials_match(request: ParsedRequest<'_>, expected_token: &str) -> bool {
    let Some(body) = request.body() else {
        return false;
    };
    let (Some(user), Some(pass)) = parse_login_form(body) else {
        return false;
    };
    let submitted = jzf407_logic::auth::basic_token(user.as_str(), pass.as_str());
    submitted.as_str() == expected_token
}

async fn complete_login(socket: &mut TcpSocket<'_>, token_seed: u64) {
    LOGIN_FAIL_COUNT.store(0, Ordering::Relaxed);
    let new_token = mint_session_token(token_seed.wrapping_add(Instant::now().as_secs()));
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
    send_and_close(socket, redirect.as_str()).await;
}

async fn reject_login(socket: &mut TcpSocket<'_>) {
    let new_fails = LOGIN_FAIL_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    LOGIN_FAIL_LAST.store(Instant::now().as_secs() as u32, Ordering::Relaxed);
    warn!("WEB: failed login attempt #{}", new_fails);

    // Preserve the response delay that slows brute-force attempts.
    Timer::after(Duration::from_millis(LOGIN_FAIL_DELAY_MS)).await;
    let message = if new_fails >= LOGIN_MAX_FAILS {
        LOGIN_LOCKED
    } else {
        LOGIN_ERROR
    };
    send_login_form(socket, HTTP_401_LOGIN, Some(message)).await;
}

async fn show_login_form(socket: &mut TcpSocket<'_>, req: &str) {
    let status = if extract_cookie(req, "jzf_session").is_some() {
        HTTP_401_EXPIRE_COOKIE
    } else {
        HTTP_401_LOGIN
    };
    send_login_form(socket, status, None).await;
}

async fn send_login_form(socket: &mut TcpSocket<'_>, status: &str, message: Option<&str>) {
    w(socket, status).await;
    w(socket, LOGIN_HEAD).await;
    if let Some(message) = message {
        w(socket, message).await;
    }
    w(socket, LOGIN_TAIL).await;
    finish_response(socket).await;
}

async fn send_and_close(socket: &mut TcpSocket<'_>, response: &str) {
    w(socket, response).await;
    finish_response(socket).await;
}

async fn finish_response(socket: &mut TcpSocket<'_>) {
    let _ = socket.flush().await;
    socket.close();
}

async fn route_authenticated(
    socket: &mut TcpSocket<'_>,
    request: ParsedRequest<'_>,
    cfg: &NetworkConfig,
    stack: Stack<'static>,
    reset_reason: ResetReason,
) {
    match (request.method, request.path) {
        ("GET", "/") => serve_dashboard(socket, cfg, stack, reset_reason).await,
        ("GET", "/events") => serve_event_stream(socket, stack).await,
        ("GET", "/login") => send_and_close(socket, HTTP_303_HOME).await,
        ("GET", "/state") => serve_state(socket, cfg, stack, reset_reason).await,
        ("POST", "/logout") => log_out(socket).await,
        ("POST", "/reboot") => reboot(socket).await,
        ("POST", "/save") => save_and_reboot(socket, request, cfg).await,
        _ => send_not_found(socket).await,
    }
}

async fn serve_dashboard(
    socket: &mut TcpSocket<'_>,
    cfg: &NetworkConfig,
    stack: Stack<'static>,
    reset_reason: ResetReason,
) {
    send_page(socket, cfg, stack, reset_reason).await;
    // Graceful FIN prevents ERR_CONNECTION_RESET in the live-state client.
    socket.close();
}

async fn serve_event_stream(socket: &mut TcpSocket<'_>, stack: Stack<'static>) {
    crate::sse::serve_events(socket, stack).await;
    socket.close();
}

async fn serve_state(
    socket: &mut TcpSocket<'_>,
    cfg: &NetworkConfig,
    stack: Stack<'static>,
    reset_reason: ResetReason,
) {
    send_state(socket, cfg, stack, reset_reason).await;
    socket.close();
}

async fn log_out(socket: &mut TcpSocket<'_>) {
    SESSION_TOKEN_HI.store(0, Ordering::Relaxed);
    SESSION_TOKEN_LO.store(0, Ordering::Relaxed);
    SESSION_CREATED.store(0, Ordering::Relaxed);
    info!("WEB: user logged out");
    send_and_close(socket, HTTP_303_LOGOUT).await;
}

async fn reboot(socket: &mut TcpSocket<'_>) {
    let _ = socket.write_all(HTTP_OK.as_bytes()).await;
    let _ = socket
        .write_all(b"<html><head><meta http-equiv=refresh content='4;url=/'></head><body><p>Rebooting... <a href=/>back in 4s</a></p></body></html>")
        .await;
    finish_response(socket).await;
    Timer::after(Duration::from_millis(300)).await;
    warn!("WEB: /reboot triggered sys_reset");
    crate::fault_marker::safe_reboot();
}

async fn save_and_reboot(
    socket: &mut TcpSocket<'_>,
    request: ParsedRequest<'_>,
    cfg: &NetworkConfig,
) {
    let Some(body) = request.body() else {
        send_and_close(socket, HTTP_400).await;
        return;
    };
    let Some(new_cfg) = parse_form(body, cfg) else {
        send_and_close(socket, HTTP_400).await;
        return;
    };

    if crate::config::save_config(&new_cfg).await.is_err() {
        warn!("WEB: EEPROM save failed");
        send_and_close(socket, HTTP_500).await;
        return;
    }

    let _ = socket.write_all(HTTP_OK.as_bytes()).await;
    let _ = socket
        .write_all(b"<html><head><meta http-equiv=refresh content='4;url=/'></head><body><p>Saved. Rebooting... <a href=/>back in 4s</a></p></body></html>")
        .await;
    finish_response(socket).await;
    Timer::after(Duration::from_millis(200)).await;
    warn!("WEB: /save triggered sys_reset");
    crate::fault_marker::safe_reboot();
}

async fn send_not_found(socket: &mut TcpSocket<'_>) {
    let _ = socket.write_all(HTTP_404.as_bytes()).await;
    socket.close();
}

fn parse_form(body: &str, current: &NetworkConfig) -> Option<NetworkConfig> {
    let mut cfg = current.clone();
    for pair in body.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("").trim();
        let val = kv.next().unwrap_or("").trim();
        let val = form_url_decode::<64>(val).ok()?;

        match key {
            "ip" => {
                cfg.ip = parse_ipv4(&val)?;
            }
            "prefix" => {
                // 0 or >32 would panic in Ipv4Cidr::new at boot → reset loop.
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
            "muser" => {
                cfg.mqtt_user = HString::try_from(val.as_str()).ok()?;
            }
            "mpass" => {
                cfg.mqtt_pass = secret_from_form(&current.mqtt_pass, val.as_str()).ok()?;
            }
            "wuser" => {
                cfg.web_user = HString::try_from(val.as_str()).ok()?;
            }
            "wpass" => {
                cfg.web_pass = secret_from_form(&current.web_pass, val.as_str()).ok()?;
            }
            _ => {}
        }
    }
    Some(cfg)
}

fn parse_login_form(body: &str) -> (Option<HString<64>>, Option<HString<64>>) {
    let mut user = None;
    let mut pass = None;
    for pair in body.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("");
        let val = kv.next().unwrap_or("");
        match key {
            "user" => user = form_url_decode::<64>(val).ok(),
            "pass" => pass = form_url_decode::<64>(val).ok(),
            _ => {}
        }
    }
    (user, pass)
}
