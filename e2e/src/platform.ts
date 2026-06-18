// OS-specific knobs. We only support Windows + Linux (RPM) — see the task scope.

import path from "node:path";
import os from "node:os";

export const isWindows = process.platform === "win32";
export const isLinux = process.platform === "linux";

/** Proxy port the *installed* service binds (voidns-proto DEFAULT_PROXY_PORT). */
export const INSTALLED_PROXY_PORT = 53;

/** High port the service binds in --mode dev so it needs no privilege. */
export const DEV_PROXY_PORT = 15353;

/**
 * Default extra-CA path the service reads at connect time
 * (voidns-core::proxy::default_extra_ca_path). Writing the mock CA here makes
 * the installed service trust the mock with no env var.
 */
export function extraCaPath(): string {
  if (isWindows) {
    const base = process.env.ProgramData ?? "C:/ProgramData";
    return path.join(base, "VoidNS", "extra-ca.pem");
  }
  return "/etc/voidns/extra-ca.pem";
}

/** Path of the installed `voidns` CLI sidecar. */
export function installedCliPath(): string {
  if (isWindows) {
    const pf = process.env["ProgramFiles"] ?? "C:/Program Files";
    return path.join(pf, "VoidNS Client", "voidns.exe");
  }
  return "/usr/bin/voidns";
}

/** Path of the installed `voidns-service` sidecar. */
export function installedServiceBinPath(): string {
  if (isWindows) {
    const pf = process.env["ProgramFiles"] ?? "C:/Program Files";
    return path.join(pf, "VoidNS Client", "voidns-service.exe");
  }
  return "/usr/bin/voidns-service";
}

/** Repo root (this file is e2e/src/platform.ts). */
export function repoRoot(): string {
  return path.resolve(import.meta.dirname, "..", "..");
}

/** Where `cargo build --release` puts binaries for --mode dev. */
export function devBin(name: string): string {
  const exe = isWindows ? `${name}.exe` : name;
  return path.join(repoRoot(), "target", "release", exe);
}

export function tmpFile(name: string): string {
  return path.join(os.tmpdir(), `voidns-e2e-${name}`);
}
