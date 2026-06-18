# voidns-e2e

Universal installer end-to-end test for the two supported targets: **Windows
(NSIS)** and **Linux (RPM)**. One TypeScript harness, run on each OS.

## What it proves

1. The Tauri installer is built.
2. It installs **headless** (`rpm -U` / `setup.exe /S`).
3. The privileged `voidns` service it registered comes up.
4. `voidns connect custom 127.0.0.1 mock.voidns.test /dns-query <port>` connects.
5. A randomized DNS query is fired at the proxy (`127.0.0.1:53`).
6. The **TypeScript mock DoH server** (`src/mock-doh.ts`, RFC 8484 over HTTP/2)
   asserts it received that exact query — i.e. the query traveled
   `client → proxy → DoH/TLS → mock`.

The mock's CA is minted at runtime (`src/certs.ts`) and written to the path the
service reads at connect time (`/etc/voidns/extra-ca.pem` /
`%ProgramData%\VoidNS\extra-ca.pem`), so the **installed** service trusts it with
no code change.

## Run it

```bash
npm ci

# Against locally-built binaries (no installer, unprivileged high port).
# Verifies the proxy↔mock DoH path on any dev machine:
cargo build --release -p voidns-service -p voidns-cli   # from repo root
npm run e2e:dev

# Against a freshly-built installer (CI does this on Windows + Fedora):
npm run e2e:install
```

### Env knobs

| Var | Meaning |
| --- | --- |
| `MOCK_PORT` | mock DoH HTTPS port (default `8853`) |
| `BUNDLE_DIR` | where to find the built `.rpm` / `setup.exe` |
| `E2E_ALLOW_MANUAL_START` | install mode: if the service isn't auto-started (e.g. a CI container with no PID1 systemd), launch the installed `voidns-service` binary directly |

`--mode dev` sets `VOIDNS_SKIP_REDIRECT=1` so it never touches the host's real
DNS; install mode on a real systemd host exercises the genuine redirect.
