#!/bin/sh
# RPM %post: register and start the privileged service (mirrors AmneziaVPN's
# deploy/data/linux/post_install.sh, translated to an RPM scriptlet).
# $1 == 1 on first install, 2 on upgrade.
set -e

# Admin-supplied extra-CA directory the proxy reads at connect time
# (/etc/voidns/extra-ca.pem). Created empty; only root can populate it.
mkdir -p /etc/voidns

# `|| true`: never fail the transaction just because the init system is
# unavailable in the build/CI environment (e.g. a non-systemd container).
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || true
    systemctl enable --now voidns.service || true
fi

exit 0
