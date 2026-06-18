#!/usr/bin/env bash
#
# One-time local install of the privileged voidns-service, modelled on
# AmneziaVPN's deploy/data/linux/post_install.sh: copy a root systemd unit and
# `systemctl enable --now` it. After this the GUI connects and changes DNS
# WITHOUT being run as root — the elevated daemon does that privileged work.
#
#   installers/linux/install-dev.sh          # sudo is requested internally
#
set -euo pipefail

APP=voidns
UNIT=$APP.service
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN_SRC="$ROOT/target/release/voidns-service"
BIN_DST=/usr/lib/$APP/voidns-service
CLI_SRC="$ROOT/target/release/voidns"
CLI_DST=/usr/bin/voidns
UNIT_SRC="$ROOT/installers/linux/$UNIT"
UNIT_DST=/etc/systemd/system/$UNIT

SUDO=""
[ "$(id -u)" -ne 0 ] && SUDO="sudo"

echo "[$APP] building service + CLI (release)…"
cargo build --release -p voidns-service -p voidns-cli --manifest-path "$ROOT/Cargo.toml"

echo "[$APP] installing service + CLI (root)…"
$SUDO systemctl stop "$UNIT" 2>/dev/null || true
$SUDO install -Dm755 "$BIN_SRC" "$BIN_DST"
$SUDO install -Dm755 "$CLI_SRC" "$CLI_DST"
$SUDO install -Dm644 "$UNIT_SRC" "$UNIT_DST"
$SUDO systemctl daemon-reload
$SUDO systemctl enable --now "$UNIT"

echo
$SUDO systemctl --no-pager --full status "$UNIT" | head -8 || true
echo
echo "[$APP] done. Drive it from a terminal:  voidns status"
echo "[$APP]       or run the GUI WITHOUT root:  ./crates/gui/run.sh"
