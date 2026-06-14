#!/bin/sh
# Debian/RPM post-install: install the binary + unit and enable the service.
# Run as root by the package manager.
set -e

install -Dm755 voidns-service /usr/lib/voidns/voidns-service
install -Dm644 voidns.service /etc/systemd/system/voidns.service

systemctl daemon-reload
systemctl enable --now voidns.service

echo "voidns service installed and started."
