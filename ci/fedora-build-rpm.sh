#!/usr/bin/env bash
# Build the Linux RPM installer inside a Fedora container. Invoked via
# `docker run ... fedora:40 bash ci/fedora-build-rpm.sh` from a self-hosted
# runner step (NOT a job `container:` — JS actions like checkout can't run in a
# job container on a docker-socket runner). The workspace is bind-mounted at the
# same path, so artifacts land back in $GITHUB_WORKSPACE on the host.
set -euxo pipefail

dnf install -y \
  git nodejs npm gcc gcc-c++ make file which curl tar xz rpm-build \
  webkit2gtk4.1-devel gtk3-devel librsvg2-devel \
  libappindicator-gtk3-devel openssl-devel

if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
# shellcheck disable=SC1091
. "$HOME/.cargo/env"

cargo build --release -p voidns-service -p voidns-cli
node crates/gui/scripts/stage-sidecars.mjs
( cd crates/gui && npm ci && npm run tauri -- build --bundles rpm )

ls -la crates/gui/src-tauri/target/release/bundle/rpm/
