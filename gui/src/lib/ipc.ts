import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type ConnState = 'disconnected' | 'connecting' | 'connected' | 'error';

export interface Status {
  state: ConnState;
  upstream?: string | null;
  listen?: string | null;
  error?: string | null;
}

export type Upstream =
  | { kind: 'voindns' }
  | { kind: 'cloudflare' }
  | { kind: 'google' }
  | { kind: 'quad9' }
  | { kind: 'custom'; ip: string; hostname: string; path: string };

export const getStatus = (): Promise<Status> => invoke<Status>('get_status');

export const connect = (upstream: Upstream): Promise<Status> =>
  invoke<Status>('connect', { upstream });

export const disconnect = (): Promise<Status> => invoke<Status>('disconnect');

/** Subscribe to live status pushed by the service. */
export const onStatus = (cb: (s: Status) => void): Promise<UnlistenFn> =>
  listen<Status>('status', (e) => cb(e.payload));
