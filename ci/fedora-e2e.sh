#!/usr/bin/env bash
# Build the RPM, then run the installer e2e inside the same Fedora container
# (real `rpm -U` install + service + DNS-through-proxy assertion). Containers
# have no PID1 systemd, so the harness launches the installed service binary
# directly via E2E_ALLOW_MANUAL_START.
set -euxo pipefail

bash ci/fedora-build-rpm.sh

export E2E_ALLOW_MANUAL_START=1
( cd e2e && npm ci && npm run e2e:install )
