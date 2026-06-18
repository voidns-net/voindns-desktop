// Headless ("--noconfirm") install / uninstall of the built package, per OS.
//   * Linux RPM  : `rpm -U` is non-interactive by nature; %post enables the
//                  systemd service.
//   * Windows NSIS: `<setup>.exe /S` runs the silent installer; the NSIS
//                  POSTINSTALL hook runs `sc create … start= auto && sc start`.

import fs from "node:fs";
import path from "node:path";
import { run, runOk } from "./exec.ts";
import { isWindows, repoRoot } from "./platform.ts";

/** Find the built installer. Override the search root with BUNDLE_DIR. */
export function findInstaller(): string {
  const kind = isWindows ? "nsis" : "rpm";
  const ext = isWindows ? ".exe" : ".rpm";
  const roots = [
    process.env.BUNDLE_DIR,
    path.join(repoRoot(), "crates", "gui", "src-tauri", "target", "release", "bundle", kind),
    path.join(repoRoot(), "target", "release", "bundle", kind),
  ].filter((x): x is string => !!x);

  for (const root of roots) {
    if (!fs.existsSync(root)) continue;
    const hit = fs
      .readdirSync(root)
      .filter((f) => f.toLowerCase().endsWith(ext))
      .sort();
    if (hit.length) return path.join(root, hit[hit.length - 1]!);
  }
  throw new Error(
    `no ${ext} installer found. Build it first (tauri build), or set BUNDLE_DIR. Looked in:\n  ${roots.join("\n  ")}`,
  );
}

/** Are we privileged enough to install system-wide? */
function isPrivileged(): boolean {
  if (isWindows) return true; // assume an elevated CI shell
  return typeof process.getuid === "function" && process.getuid() === 0;
}

function sudoize(cmd: string, args: string[]): [string, string[]] {
  if (isWindows || isPrivileged()) return [cmd, args];
  return ["sudo", ["-n", cmd, ...args]];
}

export async function install(installer: string): Promise<{ packageName?: string }> {
  if (isWindows) {
    // /S = silent. The process returns once the installer (and our hook) finish.
    await runOk(installer, ["/S"]);
    return {};
  }
  // Linux RPM.
  const nameQ = await run("rpm", ["-qp", "--queryformat", "%{NAME}", installer]);
  const packageName = nameQ.code === 0 ? nameQ.stdout.trim() : undefined;
  const [cmd, args] = sudoize("rpm", ["-Uvh", "--force", installer]);
  await runOk(cmd, args);
  return { packageName };
}

export async function uninstall(packageName?: string): Promise<void> {
  if (isWindows) {
    // Best-effort: drop the service even if the uninstaller path is unknown.
    await run("sc", ["stop", "voidns"]);
    await run("sc", ["delete", "voidns"]);
    const uninst = path.join(
      process.env["ProgramFiles"] ?? "C:/Program Files",
      "VoidNS Client",
      "uninstall.exe",
    );
    if (fs.existsSync(uninst)) await run(uninst, ["/S"]);
    return;
  }
  if (packageName) {
    const [cmd, args] = sudoize("rpm", ["-e", packageName]);
    await run(cmd, args);
  }
}
