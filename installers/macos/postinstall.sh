#!/bin/sh
# macOS .pkg postinstall: place the daemon + LaunchDaemon and bootstrap it.
set -e

install -d /Library/PrivilegedHelperTools
install -m755 voidns-service /Library/PrivilegedHelperTools/net.voidns.proxy
install -m644 net.voidns.proxy.plist /Library/LaunchDaemons/net.voidns.proxy.plist
chown root:wheel /Library/LaunchDaemons/net.voidns.proxy.plist

launchctl bootstrap system /Library/LaunchDaemons/net.voidns.proxy.plist || \
  launchctl load -w /Library/LaunchDaemons/net.voidns.proxy.plist

echo "voidns daemon installed."
