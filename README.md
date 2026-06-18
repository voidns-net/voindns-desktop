# voidns-desktop

Cross-platform desktop DNS client (Windows / Linux / macOS) in **Rust + Tauri 2 + Svelte**.
One circular **Connect / Disconnect** button (Amnezia-style). On Connect it runs a local
**DNS → DoH proxy** and redirects all system DNS through it; on Disconnect it restores the
previous DNS. A privileged **background service** does the network work and starts at boot; an
unprivileged **GUI** drives it over a local socket.

See [desktop-client-plan.md](../voidns/desktop-client-plan.md) for the full design.

## Layout

```
crates/
  voidns-proto/     shared IPC protocol (Command/Event/Status/UpstreamSel)
  voidns-core/      engine:
    proxy.rs           local DNS→DoH proxy (hickory-dns 0.26)
    redirect/          system DNS redirect per OS (linux/windows/macos)
    controller.rs      Connect/Disconnect state machine
    ipc.rs             GUI↔service local-socket protocol
  voidns-service/   privileged daemon binary (run|connect|disconnect|status)
gui/                 Tauri 2 + SvelteKit app (ConnectButton, status)
```

## Status (MVP)

| Component | State |
|---|---|
| DoH proxy core | ✅ built + integration-tested (resolves real queries over DoH) |
| IPC + controller + service | ✅ built + integration-tested (ping/status/subscribe) |
| Linux DNS redirect (systemd-resolved D-Bus + resolv.conf fallback) | ✅ built |
| Windows DNS redirect (`netsh`) | ✅ written (compiles on Windows) |
| macOS DNS redirect (`networksetup`) | ✅ written (compiles on macOS) |
| GUI (Svelte) | ✅ frontend builds |
| GUI (Tauri Rust shell) | ⚠️ code-complete; native build blocked in this env by a `tauri-utils 2.9.2` ⊥ rustc 1.95 coherence error (E0119) — see Caveats |

## Build & test

```bash
# Engine + service (verified on Linux):
cargo build
cargo test -p voidns-core            # proxy_doh + ipc_roundtrip (proxy_doh needs HTTPS egress)

# Run the service in the foreground (unprivileged dev: high port + temp socket):
VOIDNS_PORT=15353 VOIDNS_SOCK=/tmp/voidns.sock cargo run -p voidns-service

# Frontend:
cd gui && npm install && npm run build

# Full app (in a Tauri-compatible toolchain — see Caveats):
cd gui && npm run tauri dev
```

The DoH proxy binds `127.0.0.1:53` in production (privileged); set `VOIDNS_PORT` to a high port
for unprivileged runs. `VOIDNS_SOCK` overrides the IPC socket path.

## Caveats / follow-ups

- **Tauri native build (this environment).** `tauri-utils 2.9.2` triggers an `E0119` coherence
  false-positive under rustc 1.95 (and the installed Jan-2026 nightly). It is not in our code —
  the identical IPC-client code compiles fine inside `voidns-core`. Build the GUI with a
  Tauri-supported stable toolchain, or once a fixed `tauri-utils` is published.
- **Native redirect.** Windows/macOS use `netsh`/`networksetup` (Amnezia's documented fallback).
  The native `SetInterfaceDnsSettings` (windows-rs) and `SCDynamicStore` paths are tracked
  follow-ups (plan §6).
- **voidns DoH endpoint.** `UpstreamSel::Voidns` ships a placeholder bootstrap IP in
  `proxy.rs`; wire the real anycast IP before release.
- **GUI dep weight / IPC hardening.** Split an `ipc-client` crate so the GUI doesn't link
  hickory/zbus; add `SO_PEERCRED`/DACL peer checks on the socket.
