# JZF407VET6 — MQTT-контроллер на Rust + Embassy

Управление LED и реле в цеху по Ethernet через MQTT.  
Платформа: модуль **JZ-F407VET6** (STM32F407VE + DP83848 RMII PHY).  
Стек: Rust stable · Embassy · embassy-net/smoltcp · rust-mqtt · eeprom24x.

---

## Таблица пинов

| Пин  | Назначение | Примечание |
|------|-----------|------------|
| PE13 | LED1 | active-LOW (анод к VCC) |
| PE14 | LED2 | active-LOW |
| PE15 | LED3 — heartbeat | active-LOW, мигает 100 мс/7 с |
| PE10 | Кнопка S1 | pull-up R17, нажатие = LOW |
| PE11 | Кнопка S2 | pull-up R18, нажатие = LOW |
| PD4  | Реле SONGLE (P4 пин 16) | active-LOW |
| PB8  | I2C1_SCL → AT24C02 | 4.7 кОм pull-up |
| PB9  | I2C1_SDA → AT24C02 | 4.7 кОм pull-up |
| PA1,PA2,PA7,PB11–13,PC1,PC4,PC5 | ETH RMII → DP83848 | |
| PH0,PH1 | HSE 25 МГц | |

---

## Быстрый старт (macOS)

### Требования

```bash
# Rust + target
rustup target add thumbv7em-none-eabihf

# probe-rs (прошивка + RTT-логи)
cargo install probe-rs-tools --locked
```

Нужен ST-Link v2 или совместимый CMSIS-DAP адаптер.

### Сборка и прошивка

```bash
# Stage 2.1 — просто мигает LED3, проверяет тулчейн
cargo build --release

# Прошить и запустить с RTT-логами
cargo run --release
```

### Unit-тесты (нативно, без железа)

```bash
cargo test -p jzf407-logic --target aarch64-apple-darwin
```

50 тестов: debouncer (11), led_dispatch (20), config_parser (19).

---

## MQTT-топики

| Топик | Payload | Направление | Описание |
|-------|---------|-------------|----------|
| `stm32/led/1` | `1` / `0` | Subscribe | LED1 вкл/выкл |
| `stm32/led/2` | `1` / `0` | Subscribe | LED2 вкл/выкл |
| `stm32/led/all` | `1` / `0` | Subscribe | Оба LED |
| `stm32/relay` | `1` / `0` | Subscribe | Реле |
| `stm32/ping` | любой | Subscribe | Эхо → `stm32/pong` |
| `stm32/cmd/reboot` | `1` | Subscribe | Удалённый перезапуск |
| `stm32/status` | `online` / `offline` | Publish (retained) | LWT |
| `stm32/diag` | строка | Publish (retained) | Причина последнего reset |
| `stm32/heartbeat` | `1` | Publish | Каждые 10 с |
| `stm32/pong` | copy | Publish | Ответ на ping |

Payload принимается в форматах: `1`/`0`, `on`/`off`, `ON`/`OFF`, `true`/`false`.

---

## Сетевая конфигурация

По умолчанию:

```
IP:         192.168.137.2
Gateway:    192.168.137.1
Broker:     192.168.137.1:1883
Client ID:  stm32-jzf407
DHCP:       выкл
```

Настройки хранятся в AT24C02 EEPROM (offset 16, 49 байт + magic 0xC0 0x4F 0x19 0x1E).  
При первой прошивке или повреждённой EEPROM используются дефолты.

### Веб-страница конфигурации

Открыть в браузере: `http://192.168.137.2/`

```
┌─────────────────────────────────┐
│  JZF407VET6 Network Config      │
│  Device IP    [192.168.137.2  ] │
│  Prefix len   [24             ] │
│  Gateway      [192.168.137.1  ] │
│  Broker IP    [192.168.137.1  ] │
│  Broker Port  [1883           ] │
│  Client ID    [stm32-jzf407   ] │
│  ☐ Use DHCP                     │
│  [Save & Reboot]                │
│                                 │
│  [Reboot (no changes)]          │
└─────────────────────────────────┘
```

После нажатия «Save & Reboot» настройки пишутся в EEPROM и плата перезагружается через 2 с.

---

## Поведение при сбоях

| Ситуация | Поведение |
|----------|-----------|
| Обрыв кабеля / перезапуск брокера | Автоматический реконнект с задержкой 1 с |
| Питание после power-off | После старта плата сама встаёт и подключается |
| Отсутствие MQTT при нажатии кнопки | S1/S2 работают независимо |
| Retained-сообщения после реконнекта | Игнорируются первые 3 с (grace period) |
| Отсутствие ответа IWDG refresh | Аппаратный сброс через ~20 с |

Реле и LED сохраняют состояние при потере MQTT (fail-LAST, persistence в EEPROM).

---

## Reset reason

После каждой перезагрузки плата публикует причину в `stm32/diag` (retained):

| Значение | Причина |
|----------|---------|
| `power_on` | Включение питания |
| `nrst_pin` | Кнопка RESET |
| `software` | Программный сброс |
| `iwdg_timeout` | Сработал watchdog |
| `brown_out` | Просадка питания |
| `stack_overflow` | Паника / переполнение стека |
| `remote_reboot` | Команда через `stm32/cmd/reboot` |

Маркер хранится в CCMRAM по адресу `0x1000FFF0` — переживает мягкий сброс.

---

## Архитектура Embassy-задач

```
main()
 ├── heartbeat_led_task  — LED3 100 мс / 7 с, независим от сети
 ├── watchdog_task       — IWDG refresh каждые 1.5 с
 ├── buttons_task        — S1/S2 polling 10 мс, debounce 4×10 мс
 ├── mqtt_task           — MQTT клиент + reconnect loop
 └── web_task            — HTTP сервер :80
```

Shared state: `OUTPUTS` (Mutex<CriticalSectionRawMutex>) для LED.  
Кнопки → реле через `Signal<CriticalSectionRawMutex, bool>` (`RELAY_CHANGE`).

---

## Структура проекта

```
JZF407VET6/
├── Cargo.toml             — workspace + firmware crate
├── memory.x               — linker script STM32F407VE
├── build.rs
├── .cargo/config.toml     — target = thumbv7em-none-eabihf, runner = probe-rs
├── src/
│   ├── main.rs            — инициализация, spawn задач
│   ├── mqtt.rs            — MQTT клиент, топики, grace period
│   ├── outputs.rs         — Mutex-обёртка над LED пинами
│   ├── buttons.rs         — S1/S2 с async debounce
│   ├── eeprom.rs          — AT24C02 через аппаратный I2C1
│   ├── config.rs          — NetworkConfig + load/save из EEPROM
│   ├── persistence.rs     — состояние выходов в EEPROM
│   ├── heartbeat.rs       — LED3 task
│   ├── watchdog.rs        — IWDG task
│   ├── fault_marker.rs    — CCM маркер + RCC CSR
│   └── web.rs             — HTTP сервер (raw TcpSocket)
└── logic/                 — pure Rust, без embassy зависимостей
    ├── Cargo.toml
    ├── src/
    │   ├── lib.rs
    │   ├── debouncer.rs   — конечный автомат дребезга
    │   ├── led_dispatch.rs — парсинг MQTT топиков
    │   └── config.rs      — NetworkConfig: to_bytes/from_bytes, parse_ipv4/port
    └── tests/
        ├── debouncer.rs   — 11 тестов
        ├── led_dispatch.rs — 20 тестов
        └── config_parser.rs — 19 тестов
```

---

## Развёртывание на новой площадке

1. Подключить ST-Link к разъёму SWD (CN1 на плате)
2. Подключить Ethernet-кабель
3. Настроить Windows ICS или статический IP на хосте: `192.168.137.1/24`
4. Запустить mosquitto: `mosquitto -v`
5. `cargo run --release` — прошить и запустить
6. Проверить: `mosquitto_sub -t stm32/# -v`
7. Тест LED: `mosquitto_pub -t stm32/led/1 -m 1`
8. Открыть `http://192.168.137.2/` для изменения IP/брокера

---

## Troubleshooting

**Плата не появляется в сети после прошивки**
- Дождаться завершения auto-negotiation DP83848 (~3–5 с на плохом кабеле)
- Проверить LED3: если мигает — прошивка жива, ищи проблему в сети

**MQTT не подключается**
- `ping 192.168.137.2` — проверить IP
- Проверить что mosquitto слушает на 0.0.0.0, не только localhost

**Веб-страница не открывается**
- Убедиться что MQTT подключён (web_task стартует после `stack.config_v4()`)
- Проверить что браузер обращается по HTTP, не HTTPS

**Прошивка зависает, LED3 не мигает**
- IWDG сбросит плату через ~20 с, после чего `stm32/diag` = `iwdg_timeout`
- Проверить стек задач через probe-rs: `probe-rs debug --chip STM32F407VETx`

**DP83848 не поднимает линк**
- PHY адрес по умолчанию 0 — если не работает, попробовать `GenericSMI::new(1)`
- Проверить RMII пины: PA1 (REF_CLK) должен получать 50 МГц от PHY

---

## Этапы разработки

| Этап | Статус | Описание |
|------|--------|----------|
| 2.1 | ✅ | LED3 blink, тулчейн, IWDG, кнопки + реле, EEPROM, fault marker |
| 2.2 | ⬜ | Ethernet + DHCP/static IP (см. версионную заметку ниже) |
| 2.3 | ⬜ | MQTT, повтор функционала этапа 1 |
| 2.4 | ⬜ | EEPROM persistence выходов (модуль `persistence.rs` готов, не используется) |
| 2.5 | ✅ | Кнопки S1/S2 + debounce |
| 2.6 | ⬜ | Сетевой конфиг в EEPROM (read готов; web-save в 2.7) |
| 2.7 | ⬜ | Веб-страница конфигурации |
| 2.8 | ⬜ | Удалённый reboot |
| 2.9 | ✅ | Reset reason / fault marker |
| 2.10 | ⬜ | IWDG + 24-часовой стресс-тест |

## Текущее состояние (Stage 2.1)

Работает на железе:
- LED3 heartbeat 100мс/7с (PE15)
- Кнопки S1/S2 → реле PD4 (active-LOW, дебаунс 40мс)
- LED1/LED2 управляются через `OUTPUTS` (mutex), готовы к подключению MQTT
- IWDG 20 сек, refresh из watchdog_task каждые 1.5 сек
- Reset reason читается из RCC CSR + CCM marker, печатается в RTT
- Сетевой конфиг читается из AT24C02 при старте, печатается в RTT
- Все 50 unit-тестов pure-логики проходят (`cargo test -p jzf407-logic --target aarch64-apple-darwin`)

Заглушено (Stages 2.2/2.3/2.7):
- `src/mqtt.rs` — пустой таск, ждёт сигнал `RELAY_CHANGE`
- `src/web.rs` — пустой таск
- Ethernet не инициализирован в `main.rs`

### Версионная заметка по экосистеме embassy

На момент сборки актуальны:
- `embassy-executor 0.10` (фича `arch-cortex-m` переименована в `platform-cortex-m`,
  `task-arena-size-*` удалена, задачи теперь возвращают `Result<SpawnToken>`)
- `embassy-stm32 0.6` (новый `Peri<'d, T>` API, `eth::generic_smi` → `eth::GenericPhy + Sma`,
  RCC CSR флаги в F4 называются `wdgrstf`/`padrstf`, не `iwdgrstf`/`pinrstf`)
- `embassy-net 0.9` (`Stack` теперь без generic-параметра driver-а, отдельный `Runner`,
  `embassy_net::new(...) → (Stack, Runner)`)
- `embedded-io-async 0.7` — несовместимо с `picoserve 0.14` и `rust-mqtt 0.3`
  (используем `rust-mqtt 0.5`, picoserve пока пропускаем)
- `eeprom24x 0.6` использует blocking embedded-hal 0.2 — не подходит для async, поэтому
  EEPROM реализован напрямую через `i2c::I2c::new_blocking` + `blocking_write_read`

Когда `picoserve` обновится до `embedded-io-async 0.7` — заменить raw TcpSocket в `web.rs`.
