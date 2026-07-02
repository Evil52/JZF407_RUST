#!/usr/bin/env bash
# ===========================================================================
# ci/local-ci.sh — ПОЛНЫЙ пайплайн ЛОКАЛЬНО на этой машине (без GitLab/раннера)
# ===========================================================================
# Rust/Embassy-аналог CMSIS-пайплайна: те же стадии, инструменты экосистемы Rust.
#
# Стадии:
#   version   — git describe → version.txt
#   format    — cargo fmt --check (канон: rustfmt из пинов rust-toolchain)
#   build     — cargo build --release (thumbv7em-none-eabihf)
#   lint      — cargo clippy -D warnings: прошивка + host-крейт logic
#   analyze   — supply-chain security: cargo-audit (RustSec advisory DB),
#               cargo-deny (лицензии SPDX / бан-лист / advisories),
#               cargo-geiger (unsafe-метрики, информативно)
#   test      — host unit-тесты logic-крейта (./test.sh)
#   coverage  — cargo-llvm-cov по logic-крейту (HTML + summary)
#   size      — Flash/RAM бюджет ELF (cargo-size / arm-none-eabi-size)
#   docs      — cargo doc --no-deps
#   hil       — ТОЛЬКО если плата на USB: прошивка probe-rs + verify,
#               RTT smoke-тест (маркер загрузки в defmt-логе)
#
# Использование:
#   ci/local-ci.sh             — все host-стадии (без HIL)
#   ci/local-ci.sh --hil       — host-стадии + HIL (нужна подключённая плата)
#   ci/local-ci.sh --only hil  — только HIL
#   ci/local-ci.sh --help
#
# Опциональные инструменты (нет — стадия SKIP с подсказкой, не FAIL):
#   cargo install cargo-audit cargo-deny cargo-llvm-cov cargo-binutils cargo-geiger
# ===========================================================================
set -uo pipefail

# --- Корень репозитория (скрипт лежит в ci/) ------------------------------
REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# --- Настройки (переопределяются env) --------------------------------------
FW_NAME="${FW_NAME:-jzf407vet6}"
CHIP="${CHIP:-STM32F407VETx}"
FW_ELF="target/thumbv7em-none-eabihf/release/${FW_NAME}"
HOST_TRIPLE="${HOST_TRIPLE:-$(rustc -vV | sed -n 's/^host: //p')}"
# Маркер живости в RTT-логе: печатается из main() сразу после старта executor.
BOOT_MARKER="${BOOT_MARKER:-JZF407VET6 booting}"
HIL_RTT_TIMEOUT="${HIL_RTT_TIMEOUT:-20}"
# Бюджеты Flash/RAM (информативные пороги; F407VET6: 512K flash, 128K RAM).
FLASH_BUDGET_KB="${FLASH_BUDGET_KB:-400}"

# --- Цвета / счётчики ------------------------------------------------------
if [ -t 1 ]; then
  C_RED=$'\033[31m'; C_GRN=$'\033[32m'; C_YEL=$'\033[33m'
  C_BLU=$'\033[34m'; C_BLD=$'\033[1m'; C_RST=$'\033[0m'
else
  C_RED=; C_GRN=; C_YEL=; C_BLU=; C_BLD=; C_RST=
fi

PASS=0; FAIL=0; SKIP=0
declare -a RESULTS=()

stage_begin() { printf '\n%s━━━ %s ━━━%s\n' "$C_BLU$C_BLD" "$1" "$C_RST"; }
stage_pass()  { PASS=$((PASS+1)); RESULTS+=("${C_GRN}PASS${C_RST}  $1"); printf '%s[PASS] %s%s\n' "$C_GRN" "$1" "$C_RST"; }
stage_fail()  { FAIL=$((FAIL+1)); RESULTS+=("${C_RED}FAIL${C_RST}  $1"); printf '%s[FAIL] %s%s\n' "$C_RED" "$1" "$C_RST"; }
stage_skip()  { SKIP=$((SKIP+1)); RESULTS+=("${C_YEL}SKIP${C_RST}  $1 ($2)"); printf '%s[SKIP] %s (%s)%s\n' "$C_YEL" "$1" "$2" "$C_RST"; }

# Падение стадии НЕ прерывает пайплайн — гоним всё до конца, итог в сводке.
run_stage() {
  local name="$1"; shift
  stage_begin "$name"
  local rc=0
  "$@" || rc=$?
  if [ "$rc" -eq 0 ]; then stage_pass "$name"
  elif [ "$rc" -eq 99 ]; then :   # стадия сама вызвала stage_skip
  else stage_fail "$name"; fi
}

need() { command -v "$1" >/dev/null 2>&1; }

# Опциональный инструмент отсутствует → SKIP с подсказкой установки,
# НЕ FAIL: отсутствие тулзы на ноуте — не дефект кода. Канон проверит CI.
skip_missing() { stage_skip "$1" "нет $2 — установи: cargo install $3"; return 99; }

# ===========================================================================
# Реализации стадий
# ===========================================================================

do_version() {
  git describe --tags --always --dirty > version.txt 2>/dev/null \
    || git rev-parse --short HEAD > version.txt
  cat version.txt
}

st_format() {
  # Канон форматирования — rustfmt тулчейна проекта. В отличие от clang-format,
  # rustfmt стабилен между stable-релизами: расхождение = дефект кода, валим.
  cargo fmt --check
}

st_build() {
  cargo build --release || return 1
  [ -f "$FW_ELF" ] || { echo "нет $FW_ELF после сборки"; return 1; }
}

st_lint() {
  # -D warnings: любой clippy warning = FAIL (эквивалент -Werror).
  # Два прохода: прошивка (thumbv7em из .cargo/config) + logic на host-триплете
  # (иначе host-тестовый код крейта не линтится вовсе).
  cargo clippy --release -- -D warnings || return 1
  cargo clippy -p jzf407-logic --target "$HOST_TRIPLE" --all-targets -- -D warnings
}

st_analyze() {
  # Supply-chain security по практикам RustSec/OpenSSF:
  #  - cargo-audit: Cargo.lock против RustSec Advisory Database (CVE/RUSTSEC).
  #  - cargo-deny:  advisories + SPDX-лицензии + бан-лист + источники (deny.toml).
  #  - cargo-geiger: счётчик unsafe по дереву зависимостей — информативно,
  #    стадию не валит (unsafe в embedded-крейтах неизбежен, важна динамика).
  local ran=0
  if need cargo-audit; then
    ran=1
    echo "--- cargo audit (RustSec) ---"
    cargo audit || return 1
  else
    echo "⚠️  cargo-audit не найден (cargo install cargo-audit) — RustSec-проверка пропущена"
  fi
  if need cargo-deny; then
    ran=1
    echo "--- cargo deny (advisories, licenses, bans, sources) ---"
    cargo deny check || return 1
  else
    echo "⚠️  cargo-deny не найден (cargo install cargo-deny) — лицензии/баны пропущены"
  fi
  if need cargo-geiger; then
    echo "--- cargo geiger (unsafe-метрики, информативно) ---"
    cargo geiger -p jzf407-logic --target "$HOST_TRIPLE" --output-format Ascii \
      > geiger-report.txt 2>/dev/null || true
    tail -5 geiger-report.txt 2>/dev/null || true
  fi
  [ "$ran" -eq 1 ] || skip_missing analyze "cargo-audit/cargo-deny" "cargo-audit cargo-deny"
}

st_test() {
  ./test.sh
}

st_coverage() {
  need cargo-llvm-cov || skip_missing coverage cargo-llvm-cov cargo-llvm-cov || return $?
  mkdir -p coverage-html
  cargo llvm-cov -p jzf407-logic --target "$HOST_TRIPLE" \
    --html --output-dir coverage-html --summary-only 2>/dev/null \
    || cargo llvm-cov -p jzf407-logic --target "$HOST_TRIPLE" --summary-only
}

st_size() {
  # Flash/RAM бюджет: text+rodata+data → flash, data+bss → RAM.
  [ -f "$FW_ELF" ] || { echo "нет $FW_ELF — сначала стадия build"; return 1; }
  local size_tool=""
  if need cargo-size; then
    cargo size --release -- -A | tee size-report.txt
  elif need arm-none-eabi-size; then
    size_tool=arm-none-eabi-size
  elif need size; then
    size_tool=size
  else
    skip_missing size "cargo-size/arm-none-eabi-size" "cargo-binutils"; return $?
  fi
  if [ -n "$size_tool" ]; then
    "$size_tool" "$FW_ELF" | tee size-report.txt
  fi
  # Информативный порог по flash (text+data из последней строки Berkeley-формата).
  local flash_kb
  flash_kb=$(awk 'END { print int(($1 + $2) / 1024) }' < <(tail -1 size-report.txt | tr -s ' ')) || return 0
  if [ -n "${flash_kb:-}" ] && [ "$flash_kb" -gt "$FLASH_BUDGET_KB" ] 2>/dev/null; then
    echo "⚠️  flash ${flash_kb}KB > бюджета ${FLASH_BUDGET_KB}KB (информативно)"
  fi
}

st_docs() {
  # Прошивка документируется под thumbv7em (дефолтный таргет из .cargo/config),
  # RUSTDOCFLAGS -D warnings ловит битые doc-ссылки.
  RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --release || return 1
  RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p jzf407-logic --target "$HOST_TRIPLE"
}

# HIL — нужна физическая плата на USB этой машины.
st_hil() {
  need probe-rs || { echo "probe-rs не найден (cargo install probe-rs-tools)"; return 1; }
  [ -f "$FW_ELF" ] || { echo "нет $FW_ELF — сначала стадия build"; return 1; }
  # Прошивка с verify, затем reset.
  probe-rs download --chip "$CHIP" --verify "$FW_ELF" || return 1
  probe-rs reset --chip "$CHIP" || return 1
  # RTT smoke: маркер загрузки в defmt-логе за $HIL_RTT_TIMEOUT секунд.
  # attach НЕ перепрошивает; timeout убивает стрим после захвата лога.
  local rtt_log
  rtt_log=$(mktemp)
  timeout "$HIL_RTT_TIMEOUT" probe-rs attach --chip "$CHIP" "$FW_ELF" \
    > "$rtt_log" 2>&1 || true
  if grep -q "$BOOT_MARKER" "$rtt_log"; then
    echo "RTT smoke OK: найден маркер '$BOOT_MARKER'"
    grep -m5 -E "booting|Reset:|IP:|listening" "$rtt_log" || true
  else
    echo "RTT smoke FAIL: маркер '$BOOT_MARKER' не найден за ${HIL_RTT_TIMEOUT}s"
    tail -20 "$rtt_log"
    rm -f "$rtt_log"
    return 1
  fi
  rm -f "$rtt_log"
}

# Плата подключена? probe-rs list выводит строку на каждый найденный probe.
board_present() {
  need probe-rs || return 1
  probe-rs list 2>/dev/null | grep -qiE "VID|CMSIS-DAP|ST-?Link"
}

# ===========================================================================
# Разбор аргументов
# ===========================================================================
RUN_HIL=0; ONLY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --hil)  RUN_HIL=1 ;;
    --only) ONLY="${2:-}"; shift ;;
    -h|--help)
      grep '^#' "$0" | sed 's/^# \{0,1\}//' | head -30
      exit 0 ;;
    *) echo "неизвестный аргумент: $1"; exit 2 ;;
  esac
  shift
done

printf '%sЛокальный CI%s  (repo: %s)\n' "$C_BLD" "$C_RST" "$REPO_ROOT"
printf 'rustc: %s\nhost:  %s\n' "$(rustc --version)" "$HOST_TRIPLE"

stage_begin "version"
if do_version; then stage_pass "version"; else stage_fail "version"; fi

if [ -n "$ONLY" ]; then
  case "$ONLY" in
    format)   run_stage format     st_format ;;
    build)    run_stage build      st_build ;;
    lint)     run_stage lint       st_lint ;;
    analyze)  run_stage analyze    st_analyze ;;
    test)     run_stage test       st_test ;;
    coverage) run_stage coverage   st_coverage ;;
    size)     run_stage size       st_size ;;
    docs)     run_stage docs       st_docs ;;
    hil)
      if board_present; then run_stage hil st_hil
      else stage_skip hil "плата не подключена к USB"; fi ;;
    *) echo "нет такой стадии: $ONLY"; exit 2 ;;
  esac
else
  run_stage format     st_format
  run_stage build      st_build
  run_stage lint       st_lint
  run_stage analyze    st_analyze
  run_stage test       st_test
  run_stage coverage   st_coverage
  run_stage size       st_size
  run_stage docs       st_docs

  if board_present; then
    run_stage hil st_hil
  elif [ "$RUN_HIL" -eq 1 ]; then
    stage_fail hil  # явно запросили --hil, но платы нет → FAIL
  else
    stage_skip hil "плата не подключена"
  fi
fi

# ===========================================================================
# Сводка
# ===========================================================================
printf '\n%s━━━ ИТОГ ━━━%s\n' "$C_BLD" "$C_RST"
for r in "${RESULTS[@]}"; do printf '  %s\n' "$r"; done
printf '\n%spassed=%d  failed=%d  skipped=%d%s\n' "$C_BLD" "$PASS" "$FAIL" "$SKIP" "$C_RST"

[ "$FAIL" -eq 0 ]
