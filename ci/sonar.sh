#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="$REPO_ROOT/compose.sonar.yaml"
ACTION="${1:-scan}"
DOWN_VOLUMES=false
SONAR_PORT="${SONAR_PORT:-9000}"
SONAR_URL="http://127.0.0.1:${SONAR_PORT}"
COVERAGE_MIN="${COVERAGE_MIN:-80}"

if [ "$#" -gt 2 ]; then
  echo "Использование: ci/sonar.sh {up|scan|status|down [--volumes]}" >&2
  exit 2
fi
if [ -n "${2:-}" ]; then
  if [ "$ACTION" = "down" ] && [ "$2" = "--volumes" ]; then
    DOWN_VOLUMES=true
  else
    echo "Параметр '${2}' допустим только как: down --volumes" >&2
    exit 2
  fi
fi

compose() { docker compose -f "$COMPOSE_FILE" "$@"; }
need() { command -v "$1" >/dev/null 2>&1 || { echo "Не найден '$1': $2" >&2; exit 1; }; }

start_sonar() {
  need docker "установите Docker Engine/Desktop с Compose v2"
  if [ "$(uname -s)" = "Linux" ]; then
    local max_map_count file_max
    max_map_count="$(sysctl -n vm.max_map_count 2>/dev/null || echo 0)"
    file_max="$(sysctl -n fs.file-max 2>/dev/null || echo 0)"
    if [ "$max_map_count" -lt 524288 ] || [ "$file_max" -lt 131072 ]; then
      echo "Подготовьте Linux host: vm.max_map_count>=524288 и fs.file-max>=131072 (см. README)." >&2
      return 1
    fi
  fi
  compose up -d
  need curl "curl нужен для проверки готовности SonarQube"
  local attempt=0
  while [ "$attempt" -lt 90 ]; do
    if curl -fsS "$SONAR_URL/api/system/status" 2>/dev/null | grep -q '"status":"UP"'; then
      echo "SonarQube готов: $SONAR_URL"
      return 0
    fi
    attempt=$((attempt + 1))
    sleep 2
  done
  echo "SonarQube не стал готов за 3 минуты. Проверьте: docker compose -f compose.sonar.yaml logs sonarqube" >&2
  return 1
}

cd "$REPO_ROOT"

case "$ACTION" in
  up)
    start_sonar
    ;;
  status)
    need docker "установите Docker Engine/Desktop с Compose v2"
    compose ps
    ;;
  down)
    need docker "установите Docker Engine/Desktop с Compose v2"
    if [ "$DOWN_VOLUMES" = true ]; then
      compose down --volumes
      echo "SonarQube остановлен; named volumes и локальная история анализа удалены."
    else
      compose down
      echo "SonarQube остановлен; named volumes с историей сохранены."
    fi
    ;;
  scan)
    start_sonar
    need cargo "установите Rust через rustup"
    need git "установите Git и запускайте анализ из checkout репозитория"
    need sonar-scanner "установите официальный SonarScanner CLI и добавьте bin в PATH"
    branch="$(git branch --show-current)"
    if [ "$branch" != "master" ]; then
      echo "Community Build анализирует только main branch. Переключитесь на master (сейчас: '$branch')." >&2
      exit 1
    fi
    if [ -z "${SONAR_TOKEN:-}" ]; then
      echo "Задайте SONAR_TOKEN. Создать токен: $SONAR_URL -> My Account -> Security." >&2
      exit 1
    fi
    cargo llvm-cov --version >/dev/null
    host_triple="$(rustc -vV | sed -n 's/^host: //p')"
    mkdir -p target/sonar

    cargo fmt --all --check
    cargo clippy --release --message-format=json --no-deps \
      > target/sonar/clippy-firmware.json
    cargo clippy -p jzf407-logic --target "$host_triple" --all-targets --no-deps \
      --message-format=json > target/sonar/clippy-logic.json
    cargo llvm-cov -p jzf407-logic --target "$host_triple" \
      --lcov --output-path target/sonar/lcov.info --remap-path-prefix \
      --ignore-filename-regex tests --fail-under-lines "$COVERAGE_MIN"
    cargo llvm-cov report -p jzf407-logic --target "$host_triple" \
      --summary-only --ignore-filename-regex tests

    SONAR_HOST_URL="$SONAR_URL" sonar-scanner

    # Upload reports before enforcing the local no-warning policy so any
    # findings remain visible in SonarQube even when this wrapper returns FAIL.
    cargo clippy --release --no-deps -- -D warnings
    cargo clippy -p jzf407-logic --target "$host_triple" --all-targets --no-deps -- -D warnings
    echo "Анализ завершён: $SONAR_URL/dashboard?id=jzf407-rust"
    ;;
  *)
    echo "Использование: ci/sonar.sh {up|scan|status|down [--volumes]}" >&2
    exit 2
    ;;
esac
