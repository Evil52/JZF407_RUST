<img width="1280" height="964" alt="5220009269544033595" src="https://github.com/user-attachments/assets/3b42ccd4-64f0-4958-8391-096231b77592" />

<img width="1624" height="951" alt="image" src="https://github.com/user-attachments/assets/c134b543-1c5f-4325-94a0-dcb42f1c8208" />

<img width="1606" height="838" alt="image" src="https://github.com/user-attachments/assets/01725b13-57a9-4a2d-b305-89db30646d54" />


Ссылка на схему
[JZ-F407VET6.pdf](https://github.com/user-attachments/files/28059060/JZ-F407VET6.pdf)

# JZF407VET6 — Rust + Embassy MQTT Controller

Industrial LED/relay controller on the **JZ-F407VET6** module
(STM32F407VE + DP83848 RMII PHY), controllable over Ethernet via MQTT and a
built-in HTTP configuration page.

- **MCU:** STM32F407VET6 (Cortex-M4F @ 168 MHz, 512 KB Flash, 128 KB SRAM + 64 KB CCM)
- **Toolchain:** Rust (stable), `thumbv7em-none-eabihf`, `no_std`
- **Async runtime:** [Embassy](https://embassy.dev) (executor, time, net, sync)
- **Networking:** `embassy-net` / `smoltcp`, **static IPv4 + TCP only**
- **MQTT:** `rust-mqtt 0.5` (MQTT v5)
- **Storage:** AT24C02 I²C EEPROM (network config + output state)

> Status: **MVP working on hardware.** Ethernet, MQTT, HTTP UI, EEPROM
> persistence, buttons, watchdog and reset-reason reporting all function with
> **no debugger attached** — see [Hardware bring-up notes](#hardware-bring-up-notes)
> for the non-obvious fixes that made that possible.

---

## Why Rust + Embassy instead of CubeMX + C

The natural choice for an STM32 project is ST's own toolchain: CubeMX generates
the peripheral init code, HAL provides the drivers, and FreeRTOS handles
multitasking. We deliberately chose Rust + Embassy instead. Here is why.

### Memory safety — no memory leaks, no undefined behaviour, guaranteed at compile time

C gives you full control of memory, which means full responsibility for every
`malloc`/`free` pair, every pointer cast, every buffer boundary. Memory leaks,
use-after-free, null-pointer dereferences, and stack overflows are common on
microcontrollers and notoriously hard to reproduce — they often hide for weeks
until a specific message sequence or uptime threshold triggers them in
production.

Rust's ownership system makes these classes of bug **compile errors**, not
runtime surprises. The compiler tracks who owns every allocation and when it is
freed; there is no garbage collector, no runtime overhead, and no `unsafe` needed
for normal peripheral access. This firmware has **zero heap allocations** — all
buffers are statically sized and the borrow checker guarantees they are never
aliased incorrectly across tasks. A CubeMX/FreeRTOS project of equivalent
complexity would typically carry at least one latent memory bug that only
surfaces under production load.

### Fearless concurrency — no race conditions

FreeRTOS tasks share data through mutexes, queues, and volatile flags. Forgetting
a mutex, taking the wrong one, or mixing up task priorities produces data races
that are invisible in testing and catastrophic in production. Rust's type system
enforces correct sharing: a value is either owned by one task, or shared through
a synchronisation primitive, and the compiler rejects anything in between. In
this codebase `SharedOutputs`, `RELAY_CHANGE`, and `MQTT_ONLINE` are correct by
construction — the compiler would refuse to compile a version where two tasks
wrote the relay pin simultaneously without the mutex.

### Performance — zero-cost abstractions

Rust's async model compiles down to state machines with no virtual dispatch and
no heap allocation per task. Embassy's executor is interrupt-driven; when all
tasks are waiting the core enters WFI sleep immediately with no FreeRTOS tick
overhead. The result is code that is as fast as hand-written C but verifiably
safe. The entire firmware fits in **~133 KB of the 512 KB flash** and uses
**~31 KB of the 192 KB RAM**, leaving enormous headroom for future features.

### Developer experience

CubeMX generates thousands of lines of C that must be carefully preserved across
regeneration cycles. Embassy's Rust drivers are written in the same language as
the application, composable, and don't require a GUI code-generator. The `logic/`
crate runs its 50 unit tests on the host in milliseconds — no hardware, no
emulator — because Rust's `no_std` / native dual-target model makes that trivial.
Refactoring is safe: the compiler finds every broken call site.

### Summary

| | CubeMX + C + FreeRTOS | Rust + Embassy |
|---|---|---|
| Memory leaks | Possible (manual `free`) | Impossible (ownership) |
| Data races | Possible (forgotten mutex) | Compile error |
| Undefined behaviour | Frequent (C allows it) | Eliminated by design |
| Async overhead | FreeRTOS tick + context switch | Zero-cost state machines |
| Unit testing logic | Needs emulator or HW stub | Native `cargo test` |
| Code generation | CubeMX GUI, fragile | None needed |

The trade-off is a steeper learning curve — Rust's borrow checker requires
thinking about ownership upfront. Once that mental model is established the
benefits compound: every feature added to this firmware was written with
confidence that it cannot silently break existing functionality.

---

## Contents

- [Pin map](#pin-map)
- [Task architecture](#task-architecture)
- [Clock tree](#clock-tree)
- [Memory map](#memory-map)
- [Build, flash, test](#build-flash-test)
- [Network configuration](#network-configuration)
- [Web UI](#web-ui)
- [MQTT interface](#mqtt-interface)
- [Persistence & EEPROM layout](#persistence--eeprom-layout)
- [Reset-reason reporting](#reset-reason-reporting)
- [Hardware bring-up notes](#hardware-bring-up-notes)
- [Fault behavior](#fault-behavior)
- [Troubleshooting](#troubleshooting)
- [Project layout](#project-layout)

---

## Pin map

| Pin | Function | Notes |
|-----|----------|-------|
| PE13 | LED1 | **active-LOW** (drive low = on) |
| PE14 | LED2 | **active-LOW** |
| PE15 | LED3 — heartbeat | **active-LOW**, ~5 Hz proof-of-life blink |
| PE10 | Button S1 → relay ON | HW pull-up (R17); idle = High, pressed = Low |
| PE11 | Button S2 → relay OFF | HW pull-up (R18); idle = High, pressed = Low |
| PD4 | Relay (SONGLE) | **active-HIGH** (drive high = energized); off at boot |
| PB8 | I2C1_SCL → AT24C02 | external 4.7 kΩ pull-up |
| PB9 | I2C1_SDA → AT24C02 | external 4.7 kΩ pull-up |
| PA1 | ETH RMII REF_CLK | 50 MHz from PHY |
| PA2 | ETH MDIO | |
| PA7 | ETH RMII CRS_DV | |
| PB11 | ETH RMII TX_EN | |
| PB12 | ETH RMII TXD0 | |
| PB13 | ETH RMII TXD1 | |
| PC1 | ETH MDC | |
| PC4 | ETH RMII RXD0 | |
| PC5 | ETH RMII RXD1 | |
| OSC_IN/OSC_OUT | HSE 25 MHz crystal | `HseMode::Oscillator` |

> **Polarity is the #1 debugging trap here:** LEDs are active-LOW, the relay is
> active-HIGH. The relay was inverted from the original active-LOW design — if
> your relay module clicks on the wrong edge, flip `set_relay` in
> [`src/outputs.rs`](src/outputs.rs) and the boot level in
> [`src/main.rs`](src/main.rs).

---

## Task architecture

Five Embassy tasks are spawned from `main()`; none of them block the executor.

```
main()  — clock/peripheral init, EEPROM recovery, config load, then spawns:
 ├── heartbeat_led_task   LED3 ~5 Hz blink, no dependencies (executor liveness)
 ├── watchdog_task        IWDG ~20 s timeout, petted every 1.5 s
 ├── buttons_task         S1/S2 polled every 10 ms, 40 ms debounce → relay
 ├── net_task             embassy-net runner (drives the Ethernet MAC/DMA)
 ├── mqtt_task            MQTT client + reconnect loop + 10 s heartbeat publish
 └── web_task             HTTP server on :80 (raw TcpSocket)
```

**Shared state**

| Object | Type | Purpose |
|--------|------|---------|
| `OUTPUTS` | `SharedOutputs` = `Mutex<Option<OutputPins>>` | single owner of LED1/LED2/relay pins; all tasks drive them through it |
| `RELAY_CHANGE` | `Signal<bool>` | buttons_task / web_task → mqtt_task: "relay changed, publish it" |
| `MQTT_ONLINE` | `AtomicBool` | mqtt_task → web_task: true while broker connection is live |
| `EEPROM` | `Mutex<Option<Eeprom>>` | global AT24C02 handle shared by config + persistence |

**Relay control flow (and why there is no echo loop)**

```
Button S1/S2 ─┐
Web /relay/on ─┼─► OUTPUTS.set_relay() ─► pin ─► persistence.save_relay()
              │                            │
              └─► RELAY_CHANGE.signal() ───┘
                            │
                            ▼
                  mqtt_task publishes stm32/relay
```

An inbound `stm32/relay` MQTT message drives the pin **directly** and does
**not** raise `RELAY_CHANGE` — otherwise we would publish to `stm32/relay`, the
broker would echo it back (we subscribe to it), and the device would loop
forever. See `handle_event` in [`src/mqtt.rs`](src/mqtt.rs).

**Single source of truth for relay display:** the web page reads the live pin
via `OUTPUTS.get_relay()` (not the cache), so a relay toggled by button and then
by MQTT always shows the correct state.

---

## Clock tree

168 MHz from the 25 MHz HSE crystal:

```
HSE 25 MHz ─► PLL (M=25, N=336, P=2) ─► SYSCLK 168 MHz
                            └────(Q=7)─► 48 MHz (USB clock domain)
AHB  = 168 MHz (DIV1)
APB1 = 42 MHz  (DIV4)  → TIM2 kernel clock = 84 MHz
APB2 = 84 MHz  (DIV2)
```

> **Critical:** `embassy-time` **must** use `tick-hz-1_000_000`. With
> `tick-hz-1_000` at 168 MHz the TIM2 prescaler would need PSC = 83999, which
> overflows the 16-bit register and **panics inside `embassy_stm32::init()`** —
> before RTT is up, so it looks like a silent lockup. This is configured in
> [`Cargo.toml`](Cargo.toml).

---

## Memory map

`STM32F407VETx`: 512 KB Flash, 192 KB RAM total = 128 KB SRAM (112 KB SRAM1 +
16 KB SRAM2) + 64 KB CCM. See [`memory.x`](memory.x).

```
FLASH   0x0800_0000  512 KB   program + defmt strings
RAM     0x2000_0000  128 KB   .data / .bss / stack / heap-free static buffers
CCMRAM  0x1000_0000   64 KB   core-coupled (CPU only, no DMA)
  ├─ 0x1000_FF00 .. 0x1000_FFF0   panic-persist dump (240 B, UTF-8 panic message)
  └─ 0x1000_FFF0                  fault marker word (reset reason across soft reset)
```

The panic-dump region and the fault marker used to overlap; `_panic_dump_end`
is now pinned to `_fault_marker` so a panic message can never clobber the
reset-reason word.

---

## Build, flash, test

**Prerequisites**

```bash
rustup target add thumbv7em-none-eabihf
cargo install probe-rs-tools --locked      # flashing + RTT log streaming
```

A CMSIS-DAP / ST-Link v2 probe on the SWD header is required to flash.

**Firmware**

```bash
cargo build --release
cargo run   --release        # flash + attach RTT logs (probe-rs run, see .cargo/config.toml)
```

`DEFMT_LOG=debug` is set by default in [`.cargo/config.toml`](.cargo/config.toml);
override it (e.g. `DEFMT_LOG=info`) to reduce log volume.

**Host unit tests** (pure logic, no hardware)

```bash
cargo test -p jzf407-logic --target aarch64-apple-darwin
```

59 tests: `debouncer` (11), `led_dispatch` (20), `config_parser` (23), `auth`
(5). The `logic` crate has its own
[`logic/.cargo/config.toml`](logic/.cargo/config.toml) so `cargo test` inside
that directory also runs natively without the explicit `--target`.

---

## Network configuration

Defaults (used on a blank/corrupt EEPROM):

```
IP / prefix : 192.168.137.2 / 24
Gateway     : 192.168.137.1
MQTT broker : 192.168.137.1 : 1883
Client ID   : stm32-jzf407
```

The device is **static-IP only** — there is no DHCP client. Configuration is
read once at boot from the AT24C02 and applied via `Config::ipv4_static`.

**Changing the IP** (verified end-to-end): edit the field on the web page → the
form is parsed and validated (`parse_ipv4`) → written to EEPROM
(`save_config`) → device reboots → on boot `load_config` reads it back and the
stack comes up on the new address. A bad/blank EEPROM always falls back to the
defaults above, so you can never permanently lock yourself out via config.

---

## Web UI

Open `http://<device-ip>/` (default `http://192.168.137.2/`). The page streams
directly from flash-resident chunks (no large HTML buffer on the stack) and
offers live relay control plus the full network/MQTT config form (including the
MQTT and web-login credentials).

**HTTP Basic Auth.** Set a *Web Login* username/password on the page and every
request — page, `/state` poll, and all POST actions — then requires an
`Authorization: Basic` header; the browser prompts once and remembers it.
Leaving **both** web fields blank disables the prompt (open page — the default,
and how a device flashed before this feature behaves). The firmware never
decodes the header: it builds the expected `base64(user:pass)` once at boot and
compares (see [`logic/src/auth.rs`](logic/src/auth.rs), unit-tested on the host).

> ⚠️ Basic Auth over plain `http://` sends the credentials base64-encoded, **not
> encrypted** — same caveat as MQTT above. It stops casual access from the open
> internet but is not a substitute for TLS. Use a strong password and, if
> possible, restrict the forwarded port by source IP.

| Method | Path | Action |
|--------|------|--------|
| `GET` | `/` | Render status + relay control + config form |
| `POST` | `/relay/on` | Energize relay, persist, publish `stm32/relay` → redirect `/` |
| `POST` | `/relay/off` | De-energize relay, persist, publish `stm32/relay` → redirect `/` |
| `POST` | `/save` | Validate + write config to EEPROM, then reboot |
| `POST` | `/reboot` | Reboot with no config change |

Both `/save` and `/reboot` use `safe_reboot()` (see below). Invalid form input
returns `400` and does **not** write or reboot.

---

## MQTT interface

The client connects to the configured broker, sets an LWT, then subscribes to
the control topics. A 3 s grace period after connect drops retained messages so
a stale retained command can't fire on every reconnect.

**Authentication.** If an MQTT username is configured (web page → *MQTT Auth*,
persisted in EEPROM), it is sent in the CONNECT packet along with the password;
leave both blank to connect anonymously. Configure your broker to require these
credentials — e.g. mosquitto `password_file` + `allow_anonymous false` — so an
unauthenticated client cannot publish to `stm32/#`.

> ⚠️ **Plaintext over the wire.** MQTT on port 1883 sends the username and
> password unencrypted. On a trusted LAN that is fine; **exposed to the public
> internet (e.g. via port-forwarding) anyone on the path can read them.** This
> firmware does not implement MQTT-over-TLS (see audit §3.1.5) — for an
> internet-facing deployment, terminate TLS at the broker and tunnel the device
> link, or restrict the forwarded port by source IP.

| Topic | Payload | Dir | Description |
|-------|---------|-----|-------------|
| `stm32/led/1` | `1`/`0` | Sub | LED1 on/off |
| `stm32/led/2` | `1`/`0` | Sub | LED2 on/off |
| `stm32/led/all` | `1`/`0` | Sub | Both LEDs |
| `stm32/relay` | `1`/`0` | Sub | Relay on/off |
| `stm32/ping` | any | Sub | Echo → `stm32/pong` (RTT probe) |
| `stm32/cmd/reboot` | any | Sub | Remote reboot (`safe_reboot`) |
| `stm32/status` | `online`/`offline` | Pub (retained) | LWT — `offline` on disconnect |
| `stm32/diag` | string | Pub (retained) | Last reset reason |
| `stm32/heartbeat` | `1` | Pub | Every 10 s |
| `stm32/pong` | `1` | Pub | Reply to `stm32/ping` |

Boolean payloads accept `1`/`0`, `on`/`off`, `ON`/`OFF`, `true`/`false`.

**Examples**

```bash
mosquitto_sub -t 'stm32/#' -v          # watch everything
mosquitto_pub -t stm32/relay   -m 1    # relay on
mosquitto_pub -t stm32/led/all -m 0    # both LEDs off
mosquitto_pub -t stm32/ping    -m x    # expect a stm32/pong
```

---

## Persistence & EEPROM layout

The AT24C02 (256 bytes, I²C addr `0x50`) holds two independent records, each
guarded by its own 4-byte magic. Writes are byte-at-a-time with a 5 ms
write-cycle delay; reads are a single blocking transaction. A RAM cache means
EEPROM is only written when a value actually changes (saves write cycles).

**Output state — offset 0** (`src/persistence.rs`, magic `CA FE F0 0D`)

```
[0..4)  magic
[4]     relay (bit0)
[5]     led1  (bit0)
[6]     led2  (bit0)
[7]     reserved
```

**Network config — offset 16** (`logic/src/config.rs`, magic `C0 4F 19 1E`)

```
[16..20)    magic
[20..24)    device IP
[24]        prefix_len (CIDR);  [25..28) unused
[28..32)    gateway
[32..36)    broker IP
[36..38)    broker port (big-endian u16)
[38]        reserved (former DHCP flag)
[39..63)    client_id    (NUL-terminated, ≤24 bytes)
[63..95)    MQTT username (NUL-terminated, ≤32 bytes)
[95..127)   MQTT password (NUL-terminated, ≤32 bytes)
[127..159)  web username  (NUL-terminated, ≤32 bytes)
[159..191)  web password  (NUL-terminated, ≤32 bytes)
```

Validation is **magic-only** (no CRC). A wrong magic → defaults. A torn write
could in theory pass the magic check, so config is treated as advisory: anything
that fails to parse reverts to defaults rather than being trusted.

The four credential fields were appended after the original 49-byte layout. A
device flashed before this change has blank (`0xFF`) bytes in `[63..191)`; those
parse leniently to **empty** strings, so an upgrade preserves the existing
network config and boots with no credentials (anonymous MQTT + open web page) —
exactly the pre-auth behaviour.

---

## Reset-reason reporting

After every boot the firmware logs the reset reason and publishes it to
`stm32/diag` (retained). The reason combines the RCC_CSR hardware flags with a
software marker word kept in CCM RAM at `0x1000FFF0` (survives a soft reset,
cleared on power-on).

| Value | Meaning |
|-------|---------|
| `power_on` | Power-on reset |
| `nrst_pin` | NRST pin / reset button |
| `software` | Plain `sys_reset()` |
| `iwdg_timeout` | Watchdog fired |
| `brown_out` | Brown-out reset |
| `stack_overflow` | Marker set before fault |
| `malloc_failed` | Marker set before fault |
| `remote_reboot` | Marker set by `stm32/cmd/reboot` |
| `unknown` | No flag matched |

---

## Hardware bring-up notes

These are the non-obvious fixes that make the board work **without a debugger
attached**. Removing any of them re-introduces a hang or an exception that only
disappears when probe-rs holds the debug domain alive — which is exactly what
makes them so confusing to debug. Keep them.

1. **`defmt-rtt` `disable-blocking-mode`** ([`Cargo.toml`](Cargo.toml)) — in
   blocking mode `defmt` waits for a host to drain the RTT buffer; with no
   debugger that buffer fills and the firmware hangs on the first log.

2. **`DEMCR.TRCENA` set** ([`src/main.rs`](src/main.rs)) — the ETH DMA does not
   run correctly on STM32F4 with no debugger unless CoreSight trace is enabled.

3. **`AHB1LPENR` ETH low-power clock bits** (`ethlpen`/`ethrxlpen`/`ethtxlpen`,
   [`src/main.rs`](src/main.rs)) — the Embassy executor sleeps the core (WFI) when
   idle. Without these bits the ETH RX clock stops in sleep and RX dies. A
   debugger masks this by keeping clocks alive.

4. **I²C bus recovery at boot** ([`src/main.rs`](src/main.rs)) — after a soft
   reset the AT24C02 may be mid-transaction, holding SDA low; the first blocking
   I²C read would hang forever. Nine manual SCL pulses + a STOP unstick it
   before the hardware I²C peripheral is initialized.

5. **`safe_reboot()`** ([`src/fault_marker.rs`](src/fault_marker.rs)) —
   `SCB::sys_reset()` does **not** reset peripherals, so the ETH DMA keeps
   running across the reset and fires an interrupt before cortex-m-rt installs
   handlers → `DefaultHandler` exception. Fix: disable interrupts, pulse the ETH
   reset in `AHB1RSTR`, `dsb`, then `sys_reset`. **Always reboot via this, never
   bare `sys_reset()`.**

---

## Fault behavior

| Situation | Behavior |
|-----------|----------|
| Cable unplugged / broker restart | MQTT reconnects automatically (1 s backoff) |
| Power restored after outage | Boots, restores last output state, reconnects |
| Button pressed with no MQTT | S1/S2 still drive the relay locally |
| Retained messages on reconnect | Ignored for the first 3 s (grace period) |
| Firmware wedged (panic/deadlock) | IWDG resets after ~20 s → `stm32/diag = iwdg_timeout` |

Outputs are **fail-last**: relay and LED state are persisted to EEPROM and
restored on boot, so a power cycle resumes the last commanded state.

---

## Troubleshooting

**Board never appears on the network after flashing**
- Give the DP83848 a few seconds for auto-negotiation (`main` already waits 3 s).
- Check LED3: if it blinks (~5 Hz) the executor is alive — the fault is in the
  network/PHY, not the firmware.

**LED3 frozen / no logs**
- The executor is wedged. IWDG will reset in ~20 s and report `iwdg_timeout`.
- If a previous run panicked, the message is printed from `panic-persist` on the
  next boot — look for `PANIC from last run:` in the RTT log.

**MQTT won't connect**
- `ping <device-ip>` to confirm the static IP is reachable.
- Confirm the broker listens on `0.0.0.0` (not just localhost):
  `mosquitto -v` with `listener 1883 0.0.0.0`.

**Web page won't load**
- Use `http://`, not `https://`.
- `web_task` only starts serving after the network is up; wait for the
  `WEB: listening on :80` log line.

**Relay clicks on the wrong command**
- Polarity mismatch. The firmware drives the relay **active-HIGH**; if your
  module is active-LOW, invert `set_relay`/`get_relay` in
  [`src/outputs.rs`](src/outputs.rs) and the boot `Level` in
  [`src/main.rs`](src/main.rs).

---

## Project layout

```
JZF407VET6/
├── Cargo.toml              workspace + firmware crate; embedded deps gated on target_os="none"
├── memory.x                linker script (Flash/RAM/CCM regions, panic + fault marker)
├── build.rs                emits link args for memory.x
├── .cargo/config.toml      target = thumbv7em-none-eabihf, runner = probe-rs run
├── src/                    firmware (no_std, target-only)
│   ├── main.rs             clock/peripheral init, bring-up fixes, task spawning
│   ├── outputs.rs          SharedOutputs mutex over LED1/LED2/relay (polarity lives here)
│   ├── buttons.rs          S1/S2 polling + debounce → relay
│   ├── mqtt.rs             MQTT client, topic handling, reconnect, grace period
│   ├── web.rs              HTTP server (raw TcpSocket), config form, relay control
│   ├── config.rs           NetworkConfig EEPROM load/save (firmware side)
│   ├── persistence.rs      output-state RAM cache + EEPROM flush
│   ├── eeprom.rs           AT24C02 driver over blocking I2C1
│   ├── heartbeat.rs        LED3 proof-of-life task
│   ├── watchdog.rs         IWDG refresh task
│   ├── fault_marker.rs     reset-reason marker + safe_reboot()
│   └── net.rs              embassy-net runner task (concrete type wrapper)
└── logic/                  pure logic crate (no_std + host-testable, no Embassy)
    ├── src/
    │   ├── debouncer.rs    sample-history debounce state machine
    │   ├── led_dispatch.rs MQTT topic+payload → OutputCmd
    │   ├── auth.rs         HTTP Basic Auth token builder (base64 of user:pass)
    │   └── config.rs       NetworkConfig (de)serialization + parse_ipv4/parse_port
    └── tests/              59 native unit tests
```

The `logic` crate deliberately has **no Embassy or hardware dependencies** so
its parsing/state-machine code is unit-tested on the host. Anything touching a
peripheral lives in `src/`.
