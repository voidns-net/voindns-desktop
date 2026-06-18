#!/bin/sh
# RPM %preun: stop+disable the service on real uninstall only.
# $1 == 0 on uninstall, 1 on upgrade (keep it running across upgrades).
set -e

if [ "$1" = "0" ] && command -v systemctl >/dev/null 2>&1; then
    systemctl disable --now voidns.service || true
fi

exit 0
