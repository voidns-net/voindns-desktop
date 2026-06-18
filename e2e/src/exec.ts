// Tiny child-process helpers (no shell, explicit argv).

import { spawn } from "node:child_process";

export interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
}

/** Run a command to completion, capturing output. Never throws on non-zero. */
export function run(
  cmd: string,
  args: string[],
  opts: { env?: NodeJS.ProcessEnv; cwd?: string } = {},
): Promise<RunResult> {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, {
      env: { ...process.env, ...opts.env },
      cwd: opts.cwd,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => (stdout += d.toString()));
    child.stderr.on("data", (d) => (stderr += d.toString()));
    child.once("error", reject);
    child.once("close", (code) => resolve({ code: code ?? -1, stdout, stderr }));
  });
}

/** Like `run`, but rejects on a non-zero exit (for steps that must succeed). */
export async function runOk(
  cmd: string,
  args: string[],
  opts: { env?: NodeJS.ProcessEnv; cwd?: string } = {},
): Promise<RunResult> {
  const r = await run(cmd, args, opts);
  if (r.code !== 0) {
    throw new Error(
      `command failed (${r.code}): ${cmd} ${args.join(" ")}\n${r.stdout}\n${r.stderr}`,
    );
  }
  return r;
}

export const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
