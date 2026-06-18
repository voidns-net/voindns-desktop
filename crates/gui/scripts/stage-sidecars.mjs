// Stage the workspace binaries as Tauri externalBin sidecars.
//
// Tauri requires sidecars to be named `<name>-<target-triple>[.exe]` next to the
// path listed in tauri.conf.json `bundle.externalBin`. We build `voidns-service`
// and `voidns` in the root workspace, then copy them here with the triple suffix.
// Run AFTER `cargo build --release -p voidns-service -p voidns-cli`.

import { execSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const triple = execSync("rustc -vV")
  .toString()
  .split("\n")
  .find((l) => l.startsWith("host:"))
  ?.slice("host:".length)
  .trim();

if (!triple) {
  console.error("could not determine host target triple from `rustc -vV`");
  process.exit(1);
}

const ext = process.platform === "win32" ? ".exe" : "";
const guiDir = path.resolve(import.meta.dirname, "..");
const repoRoot = path.resolve(guiDir, "..", "..");
const releaseDir = path.join(repoRoot, "target", "release");
const destDir = path.join(guiDir, "src-tauri", "binaries");

fs.mkdirSync(destDir, { recursive: true });

for (const name of ["voidns-service", "voidns"]) {
  const src = path.join(releaseDir, `${name}${ext}`);
  const dst = path.join(destDir, `${name}-${triple}${ext}`);
  if (!fs.existsSync(src)) {
    console.error(`missing ${src} — run \`cargo build --release\` first`);
    process.exit(1);
  }
  fs.copyFileSync(src, dst);
  console.log(`staged ${path.relative(repoRoot, dst)}`);
}
