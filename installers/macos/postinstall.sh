#!/bin/sh
# macOS .pkg postinstall: place the daemon + LaunchDaemon and bootstrap it.
set -e

install -d /Library/PrivilegedHelperTools
install -m755 voidns-service /Library/PrivilegedHelperTools/net.voidns.proxy
# CLI twin of the GUI — drive the daemon from a terminal (`voidns status`).
[ -f voidns ] && install -m755 voidns /usr/local/bin/voidns
install -m644 net.voidns.proxy.plist /Library/LaunchDaemons/net.voidns.proxy.plist
chown root:wheel /Library/LaunchDaemons/net.voidns.proxy.plist

launchctl bootstrap system /Library/LaunchDaemons/net.voidns.proxy.plist || \
  launchctl load -w /Library/LaunchDaemons/net.voidns.proxy.plist

echo "voidns daemon installed."
