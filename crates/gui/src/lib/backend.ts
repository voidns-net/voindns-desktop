// Thin typed wrapper around the Tauri commands exposed by src-tauri/src/lib.rs.
// When NOT running inside Tauri (vite dev / browser preview) every call is a
// no-op stub and the UI falls back to its built-in simulation — so the design
// stays previewable in a plain browser.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ConnState = "disconnected" | "connecting" | "connected" | "error";

export interface Status {
  state: ConnState;
  upstream: string | null;
  listen: string | null;
  error: string | null;
}

// Mirrors voidns_proto::UpstreamSel (#[serde(tag = "kind", rename_all = "snake_case")]).
export type UpstreamSel =
  | { kind: "voidns" }
  | { kind: "cloudflare" }
  | { kind: "google" }
  | { kind: "quad9" }
  | { kind: "custom"; ip: string; hostname: string; path: string };

const STATUS_EVENT = "voidns://status";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Show the simulated Dev provider in debug builds (and in browser preview). */
export async function isDevBuild(): Promise<boolean> {
  if (!isTauri()) return true;
  try {
    return await invoke<boolean>("is_dev");
  } catch {
    return false;
  }
}

export async function connect(upstream: UpstreamSel): Promise<Status> {
  return invoke<Status>("connect", { upstream });
}

export async function disconnect(): Promise<Status> {
  return invoke<Status>("disconnect");
}

export async function getStatus(): Promise<Status> {
  return invoke<Status>("get_status");
}

/** Subscribe to live status pushed by the service. Returns an unlisten fn. */
export async function onStatus(cb: (s: Status) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  return listen<Status>(STATUS_EVENT, (e) => cb(e.payload));
}
