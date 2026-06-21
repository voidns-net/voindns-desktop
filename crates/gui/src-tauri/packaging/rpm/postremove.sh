#!/bin/sh
# RPM %postun: reload units after the unit file is removed.
set -e

if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || true
fi

exit 0
