// Real system e2e for the voidns desktop daemon, one script for all 3 OSes.
//
// Flow (a–d from the spec): install the REAL service (no GUI) → CLI `connect`
// (the same IPC the GUI uses) → resolve a name through the SYSTEM resolver →
// assert the query reached our local mock DoH server through the redirect.
//
// Why one Node process instead of separate workflow steps: the mock DoH server
// must stay alive for the whole flow. On Linux/macOS a `node … &` background
// process survives across GitHub-Actions steps, but on Windows a process
// started in one step is torn down before the next — so the mock was dead by
// the time the daemon dialed it and every upstream lookup ServFailed. Keeping
// the mock as a child of this single long-lived process fixes that, and folds
// three near-identical OS jobs into a handful of small per-OS branches.
//
// Run: node --experimental-strip-types crates/voidns-core/tests/e2e/run-e2e.ts

import { spawn, spawnSync } from 'node:child_process'
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'

const PLAT = process.platform // 'linux' | 'darwin' | 'win32'
const isWin = PLAT === 'win32'
const WS = process.env.GITHUB_WORKSPACE ?? process.cwd()
const REL = join(WS, 'target', 'release')
const MOCK_DIR = join(WS, 'crates', 'voidns-core', 'tests', 'doh-mock')

// The installed CLI and the path the daemon reads an extra CA from.
const CLI = isWin ? 'C:\\Program Files\\VoidNS\\voidns.exe' : 'voidns'
const CA_DEST = isWin
  ? join(process.env.ProgramData ?? 'C:\\ProgramData', 'VoidNS', 'extra-ca.pem')
  : '/etc/voidns/extra-ca.pem'
// The macOS LaunchDaemon plist runs the service with this socket; the CLI must
// match it. Linux/Windows use the built-in defaults.
if (PLAT === 'darwin') process.env.VOIDNS_SOCK = '/var/run/net.voidns.proxy.sock'

type RunOpts = { sudo?: boolean; check?: boolean; cwd?: string }
type RunResult = { code: number; out: string }

/** Run a command synchronously, echoing its output. Throws on non-zero unless check:false. */
function run(cmd: string, args: string[], opts: RunOpts = {}): RunResult {
  let c = cmd
  let a = args
  if (opts.sudo && !isWin) {
    a = [cmd, ...args]
    c = 'sudo'
  }
  console.log(`+ ${c} ${a.join(' ')}`)
  const r = spawnSync(c, a, { encoding: 'utf8', cwd: opts.cwd })
  const out = (r.stdout ?? '') + (r.stderr ?? '')
  if (out.trim()) console.log(out.trim())
  if (opts.check !== false && r.status !== 0) {
    throw new Error(`command failed (exit ${r.status}): ${c} ${a.join(' ')}`)
  }
  return { code: r.status ?? -1, out }
}

/** Poll `fn` until it returns a truthy value or `ms` elapses. */
async function waitFor<T>(fn: () => T | undefined, ms: number, label: string): Promise<T> {
  const deadline = Date.now() + ms
  for (;;) {
    const v = fn()
    if (v) return v
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${label}`)
    await sleep(200)
  }
}

async function main(): Promise<void> {
  const dir = mkdtempSync(join(tmpdir(), 'voidns-e2e-'))

  // --- generate a mock CA + a localhost leaf (no shell → no MSYS path mangling) ---
  const ca = join(dir, 'ca.crt')
  const caKey = join(dir, 'ca.key')
  const leaf = join(dir, 'leaf.crt')
  const leafKey = join(dir, 'leaf.key')
  const csr = join(dir, 'leaf.csr')
  const ext = join(dir, 'san.cnf')
  writeFileSync(ext, 'subjectAltName=DNS:localhost,IP:127.0.0.1\n')
  run('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', caKey, '-out', ca, '-days', '2', '-subj', '/CN=voidns-e2e-ca'])
  run('openssl', ['req', '-newkey', 'rsa:2048', '-nodes', '-keyout', leafKey, '-out', csr, '-subj', '/CN=localhost'])
  run('openssl', ['x509', '-req', '-in', csr, '-CA', ca, '-CAkey', caKey, '-CAcreateserial', '-out', leaf, '-days', '2', '-extfile', ext])

  // --- start the mock DoH server as a child that lives for the whole run ---
  let mockOut = ''
  const mock = spawn(process.execPath, ['mock-doh.mjs', leaf, leafKey, '0'], { cwd: MOCK_DIR })
  mock.stdout.on('data', (d) => (mockOut += d))
  mock.stderr.on('data', (d) => {
    mockOut += d
    process.stdout.write(`[mock] ${d}`)
  })
  mock.on('exit', (code) => console.log(`[mock] exited (code ${code})`))

  try {
    const port = await waitFor(() => /^PORT=(\d+)/m.exec(mockOut)?.[1], 10_000, 'mock PORT')
    console.log(`mock listening on 127.0.0.1:${port}`)

    // --- place the CA where the daemon reads it (offline rustls trust) ---
    if (isWin) {
      mkdirSync(dirname(CA_DEST), { recursive: true })
      copyFileSync(ca, CA_DEST)
    } else {
      run('mkdir', ['-p', dirname(CA_DEST)], { sudo: true })
      run('cp', [ca, CA_DEST], { sudo: true })
    }

    // --- a. install the real service (no GUI) ---
    if (PLAT === 'linux') {
      run('bash', [join(WS, 'installers', 'linux', 'install-dev.sh')], { sudo: true })
    } else if (PLAT === 'darwin') {
      const stage = join(dir, 'pkg')
      mkdirSync(stage)
      for (const f of ['voidns-service', 'voidns']) copyFileSync(join(REL, f), join(stage, f))
      copyFileSync(join(WS, 'installers', 'macos', 'net.voidns.proxy.plist'), join(stage, 'net.voidns.proxy.plist'))
      copyFileSync(join(WS, 'installers', 'macos', 'postinstall.sh'), join(stage, 'postinstall.sh'))
      run('sh', ['./postinstall.sh'], { sudo: true, cwd: stage })
    } else {
      run('powershell', ['-NoProfile', '-File', join(WS, 'installers', 'windows', 'install-service.ps1'), '-Action', 'install', '-SourceDir', REL])
    }

    // Wait until the freshly-installed service answers the CLI.
    await waitFor(() => (run(CLI, ['status'], { check: false }).code === 0 ? true : undefined), 10_000, 'service to come up')
    run(CLI, ['status'])

    // --- b. connect to the mock through the same IPC the GUI uses ---
    run(CLI, ['connect', 'custom', '127.0.0.1', 'localhost', '/dns-query', port])

    // --- c. resolve through the SYSTEM resolver (OS-native) ---
    let answer = ''
    if (PLAT === 'linux') {
      run('resolvectl', ['flush-caches'], { sudo: true, check: false })
      answer += run('resolvectl', ['query', 'example.com'], { check: false }).out
      answer += run('dig', ['+short', '+tries=2', '+time=3', 'example.com', 'A'], { check: false }).out
    } else if (PLAT === 'darwin') {
      run('dscacheutil', ['-flushcache'], { sudo: true, check: false })
      run('killall', ['-HUP', 'mDNSResponder'], { sudo: true, check: false })
      answer = run('dscacheutil', ['-q', 'host', '-a', 'name', 'example.com'], { check: false }).out
    } else {
      run('ipconfig', ['/flushdns'], { check: false })
      answer = run('powershell', ['-NoProfile', '-Command',
        "(Resolve-DnsName -Name example.com -Type A -DnsOnly -EA SilentlyContinue | Where-Object {$_.Type -eq 'A'} | Select-Object -First 1).IPAddress"],
        { check: false }).out
    }
    console.log(`system resolve example.com => ${answer.trim().split('\n').join(' / ') || '(empty)'}`)

    // --- d. the query must have reached the mock through the redirect ---
    console.log('---- mock output ----')
    console.log(mockOut.trim())
    if (!/^QUERY=example\.com/m.test(mockOut)) throw new Error('FAIL: mock never received the query')
    if (!answer.includes('127.0.0.1')) throw new Error(`FAIL: resolver did not return 127.0.0.1 (got: ${answer.trim()})`)
    console.log('OK: install → CLI connect → system resolve → mock caught the query')
  } finally {
    run(CLI, ['disconnect'], { check: false })
    if (PLAT === 'linux') run('bash', [join(WS, 'installers', 'linux', 'uninstall-dev.sh')], { sudo: true, check: false })
    else if (PLAT === 'darwin') run('launchctl', ['bootout', 'system', '/Library/LaunchDaemons/net.voidns.proxy.plist'], { sudo: true, check: false })
    else run('powershell', ['-NoProfile', '-File', join(WS, 'installers', 'windows', 'install-service.ps1'), '-Action', 'uninstall'], { check: false })
    mock.kill()
  }
}

main().catch((e) => {
  console.error(String(e?.stack ?? e))
  process.exit(1)
})
