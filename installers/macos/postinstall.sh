#!/bin/sh
# macOS .pkg postinstall: place the daemon + LaunchDaemon and bootstrap it.
set -e

install -d /Library/PrivilegedHelperTools
install -m755 voindns-service /Library/PrivilegedHelperTools/net.voindns.proxy
install -m644 net.voindns.proxy.plist /Library/LaunchDaemons/net.voindns.proxy.plist
chown root:wheel /Library/LaunchDaemons/net.voindns.proxy.plist

launchctl bootstrap system /Library/LaunchDaemons/net.voindns.proxy.plist || \
  launchctl load -w /Library/LaunchDaemons/net.voindns.proxy.plist

echo "voindns daemon installed."
