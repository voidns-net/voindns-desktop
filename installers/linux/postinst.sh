#!/bin/sh
# Debian/RPM post-install: install the binary + unit and enable the service.
# Run as root by the package manager.
set -e

install -Dm755 voindns-service /usr/lib/voindns/voindns-service
install -Dm644 voindns.service /etc/systemd/system/voindns.service

systemctl daemon-reload
systemctl enable --now voindns.service

echo "voindns service installed and started."
