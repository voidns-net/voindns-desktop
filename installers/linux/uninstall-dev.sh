#!/usr/bin/env bash
#
# Undo install-dev.sh: stop/disable the voidns-service unit and remove it.
# Modelled on AmneziaVPN's deploy/data/linux/post_uninstall.sh.
#
#   installers/linux/uninstall-dev.sh
#
set -euo pipefail

APP=voidns
UNIT=$APP.service
BIN_DST=/usr/lib/$APP/voidns-service
UNIT_DST=/etc/systemd/system/$UNIT

SUDO=""
[ "$(id -u)" -ne 0 ] && SUDO="sudo"

echo "[$APP] stopping + disabling service…"
$SUDO systemctl stop "$UNIT" 2>/dev/null || true
$SUDO systemctl disable "$UNIT" 2>/dev/null || true

$SUDO rm -f "$UNIT_DST"
$SUDO rm -f "$BIN_DST"
$SUDO rmdir /usr/lib/$APP 2>/dev/null || true
$SUDO systemctl daemon-reload

echo "[$APP] removed. (DNS is restored by the service on stop.)"
