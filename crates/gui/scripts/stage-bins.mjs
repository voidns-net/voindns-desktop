// Stage the privileged service + CLI as Tauri "externalBin" sidecars so the GUI
// installer ships them next to voidns-gui(.exe). Tauri requires each sidecar to
// be named `<base>-<target-triple><ext>`; at install time it strips the triple,
// so `binaries/voidns-service-<triple>.exe` lands as `$INSTDIR\voidns-service.exe`
// — exactly what installers/windows/install-service.nsh registers with `sc`.
//
// Runs from the GUI crate (crates/gui) before every `tauri build`/`tauri dev`
// (see tauri.conf.json beforeBuild/DevCommand). Builds the release binaries only
// if they are missing, then copies them in with the triple suffix.
import { execFileSync } from 'node:child_process'
import { mkdirSync, copyFileSync, existsSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const guiDir = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const workspace = resolve(guiDir, '../..')
const manifest = resolve(workspace, 'Cargo.toml')
const releaseDir = resolve(workspace, 'target/release')
const outDir = resolve(guiDir, 'src-tauri/binaries')

// Host target triple, e.g. x86_64-pc-windows-msvc / x86_64-unknown-linux-gnu.
const triple = execFileSync('rustc', ['-vV'], { encoding: 'utf8' })
  .split('\n')
  .find((l) => l.startsWith('host:'))
  ?.slice('host:'.length)
  .trim()
if (!triple) throw new Error('could not determine host target triple from `rustc -vV`')

const ext = process.platform === 'win32' ? '.exe' : ''
// crate bin name -> shipped sidecar base name
const bins = [
  { src: `voidns-service${ext}`, base: 'voidns-service', pkg: 'voidns-service' },
  { src: `voidns${ext}`, base: 'voidns', pkg: 'voidns-cli' },
]

const missing = bins.filter((b) => !existsSync(resolve(releaseDir, b.src)))
if (missing.length) {
  const args = ['build', '--release', '--locked', '--manifest-path', manifest]
  for (const b of missing) args.push('-p', b.pkg)
  console.log(`[stage-bins] building: ${missing.map((b) => b.pkg).join(', ')}`)
  execFileSync('cargo', args, { stdio: 'inherit' })
}

mkdirSync(outDir, { recursive: true })
for (const b of bins) {
  const from = resolve(releaseDir, b.src)
  const to = resolve(outDir, `${b.base}-${triple}${ext}`)
  copyFileSync(from, to)
  console.log(`[stage-bins] ${b.src} -> ${to}`)
}
