#!/usr/bin/env bash
#
# Запуск VoidNS Client (desktop GUI, Tauri 2 + Svelte 5).
#
#   ./run.sh            запустить standalone-бинарь (собрать, если нет)
#   ./run.sh --build    принудительно пересобрать, затем запустить
#   ./run.sh --dev      дев-режим (vite + окно)
#
set -euo pipefail

# Скрипт лежит в самом крейте GUI — app == директория скрипта.
APP="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$APP/src-tauri/target/release/voidns-gui"
MODE="${1:-run}"

cd "$APP"

if [ ! -d node_modules ]; then
  echo "[voidns-gui] npm install…"
  npm install
fi

if [ "$MODE" = "--dev" ]; then
  echo "[voidns-gui] dev-режим…"
  exec npm run tauri dev
fi

if [ ! -x "$BIN" ] || [ "$MODE" = "--build" ]; then
  echo "[voidns-gui] сборка standalone-бинаря…"
  npx tauri build --no-bundle
fi

echo "[voidns-gui] запуск: $BIN"
exec env WEBKIT_DISABLE_DMABUF_RENDERER=1 "$BIN"
