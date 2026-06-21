// Universal installer e2e — one harness, both supported OSes (Windows + Linux
// RPM). Implements the 6-step flow from the task:
//
//   1. (installers built beforehand by installers.yml / the workflow step)
//   2. install the package headless ("--noconfirm")
//   3. wait for the service to come up
//   4. `voidns connect custom 127.0.0.1 mock.voidns.test /dns-query <port>`
//   5. fire a DNS query at the proxy
//   6. assert it reached the mock DoH server
//
// `--mode dev` runs the same 4–6 against locally-built binaries (no installer,
// unprivileged high port) so the DNS/TLS path is verifiable without packaging.

import fs from "node:fs";
import path from "node:path";
import { spawn, type ChildProcess } from "node:child_process";

import { mintCerts, MOCK_DOH_HOST } from "./certs.ts";
import { startMockDoh, type MockDoh } from "./mock-doh.ts";
import { probe, randomProbeName } from "./dns-probe.ts";
import { findInstaller, install, uninstall } from "./install.ts";
import { run, runOk, sleep } from "./exec.ts";
import {
  DEV_PROXY_PORT,
  INSTALLED_PROXY_PORT,
  devBin,
  extraCaPath,
  installedCliPath,
  installedServiceBinPath,
  isWindows,
  tmpFile,
} from "./platform.ts";

type Mode = "install" | "dev";

function parseMode(argv: string[]): Mode {
  const i = argv.indexOf("--mode");
  const v = i >= 0 ? argv[i + 1] : process.env.E2E_MODE;
  if (v === "dev" || v === "install") return v;
  return "install";
}

function log(step: string, msg: string) {
  console.log(`[e2e] ${step.padEnd(10)} ${msg}`);
}

/** Write the mock CA where the service will read it, elevating if needed. */
async function writeCa(target: string, pem: string): Promise<void> {
  const dir = path.dirname(target);
  try {
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(target, pem);
    return;
  } catch {
    if (isWindows) throw new Error(`cannot write CA to ${target} (run elevated)`);
  }
  // Non-root Linux: shell out via sudo.
  await runOk("sudo", ["-n", "mkdir", "-p", dir]);
  const tmp = tmpFile("ca-stage.pem");
  fs.writeFileSync(tmp, pem);
  await runOk("sudo", ["-n", "cp", tmp, target]);
}

/** Poll `voidns ping` until the service answers, or fail after `timeoutMs`. */
async function waitForService(
  cli: string,
  env: NodeJS.ProcessEnv,
  timeoutMs: number,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let last = "";
  while (Date.now() < deadline) {
    const r = await run(cli, ["ping"], { env });
    if (r.code === 0 && /pong/i.test(r.stdout)) return;
    last = `${r.stdout}${r.stderr}`.trim();
    await sleep(500);
  }
  throw new Error(`service did not become reachable in ${timeoutMs}ms (last: ${last || "no reply"})`);
}

async function main(): Promise<number> {
  const mode = parseMode(process.argv.slice(2));
  const mockPort = Number(process.env.MOCK_PORT ?? 8853);
  const proxyPort = mode === "dev" ? DEV_PROXY_PORT : INSTALLED_PROXY_PORT;

  log("mode", `${mode} (proxy port ${proxyPort}, mock DoH https://127.0.0.1:${mockPort}/dns-query)`);

  // Step 1 happens in CI before us; here we set up the mock upstream.
  const certs = mintCerts(MOCK_DOH_HOST);
  const mock: MockDoh = await startMockDoh({
    cert: certs.certPem,
    key: certs.keyPem,
    port: mockPort,
  });
  log("mock", "DoH server listening");

  let serviceChild: ChildProcess | undefined;
  let packageName: string | undefined;
  let cli = "";
  let cliEnv: NodeJS.ProcessEnv = {};

  try {
    if (mode === "dev") {
      const caFile = tmpFile("ca.pem");
      fs.writeFileSync(caFile, certs.caPem);
      const sock = isWindows ? "voidns-e2e" : tmpFile("control.sock");
      cliEnv = {
        VOIDNS_PORT: String(proxyPort),
        VOIDNS_SOCK: sock,
        VOIDNS_EXTRA_CA_FILE: caFile,
        VOIDNS_LOG_FILE: tmpFile("service.log"),
        // Dev mode is unprivileged: start the proxy but don't touch system DNS
        // (we query its loopback port directly). The privileged install-mode run
        // leaves this unset and exercises the real redirect.
        VOIDNS_SKIP_REDIRECT: "1",
        RUST_LOG: process.env.RUST_LOG ?? "info",
      };
      const svc = devBin("voidns-service");
      if (!fs.existsSync(svc)) {
        throw new Error(`missing ${svc} — run \`cargo build --release\` first`);
      }
      cli = devBin("voidns");
      log("service", `spawning ${svc} run`);
      serviceChild = spawn(svc, ["run"], {
        env: { ...process.env, ...cliEnv },
        stdio: "ignore",
      });
    } else {
      // Step 2: write CA the installed service will trust, then install.
      await writeCa(extraCaPath(), certs.caPem);
      log("ca", `wrote mock CA to ${extraCaPath()}`);
      const installer = findInstaller();
      log("install", `installing ${path.basename(installer)} (headless)`);
      ({ packageName } = await install(installer));
      cli = installedCliPath();
    }

    // Step 3: wait for the service the installer started.
    try {
      await waitForService(cli, cliEnv, 60_000);
    } catch (e) {
      // CI fallback: GitHub Linux containers don't run systemd as PID1, so
      // `systemctl enable --now` in %post is a no-op. With E2E_ALLOW_MANUAL_START
      // we launch the *installed* service binary ourselves — still exercising the
      // packaged artifact, the installed CA path and the real :53 port. Skip the
      // redirect (the container's /etc/resolv.conf is a read-only bind mount; the
      // assertion queries the proxy directly anyway).
      if (mode === "install" && process.env.E2E_ALLOW_MANUAL_START && !isWindows) {
        log("service", "not auto-started; launching installed binary (manual fallback)");
        serviceChild = spawn(installedServiceBinPath(), ["run"], {
          env: { ...process.env, VOIDNS_SKIP_REDIRECT: "1" },
          stdio: "ignore",
        });
        await waitForService(cli, cliEnv, 30_000);
      } else {
        throw e;
      }
    }
    log("service", "reachable (pong)");

    // Step 4: connect to the mock DoH via a custom upstream.
    log("connect", `voidns connect custom 127.0.0.1 ${MOCK_DOH_HOST} /dns-query ${mockPort}`);
    await runOk(
      cli,
      ["connect", "custom", "127.0.0.1", MOCK_DOH_HOST, "/dns-query", String(mockPort)],
      { env: cliEnv },
    );

    // Step 5: query the proxy.
    const name = randomProbeName();
    log("query", `A ${name} -> 127.0.0.1:${proxyPort}`);
    const result = await probe({ host: "127.0.0.1", port: proxyPort, name, timeoutMs: 5000 });

    // Step 6: assert the mock received it.
    const reached = await mock.waitFor(name, 10_000);
    if (!reached) {
      throw new Error(`FAIL: mock DoH never received ${name}. seen=${JSON.stringify(mock.seen)}`);
    }
    log("assert", `PASS — mock received ${name}; proxy answered ${JSON.stringify(result.answers)}`);

    await run(cli, ["disconnect"], { env: cliEnv });
    return 0;
  } catch (err) {
    console.error(`[e2e] ERROR ${err instanceof Error ? err.message : String(err)}`);
    return 1;
  } finally {
    if (serviceChild) {
      serviceChild.kill(isWindows ? undefined : "SIGINT");
    }
    if (mode === "install") {
      await uninstall(packageName).catch(() => {});
    }
    await mock.close();
  }
}

main().then((code) => process.exit(code));
