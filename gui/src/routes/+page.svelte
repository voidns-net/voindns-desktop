<script lang="ts">
  import { onMount } from 'svelte';
  import ConnectButton from '$lib/ConnectButton.svelte';
  import { getStatus, connect, disconnect, onStatus, type Status } from '$lib/ipc';

  let status = $state<Status>({ state: 'disconnected' });
  let busy = $state(false);
  let upstreamKind = $state<'voindns' | 'cloudflare' | 'google' | 'quad9'>('cloudflare');

  const btnState = $derived(
    status.state === 'connected'
      ? 'connected'
      : status.state === 'connecting'
        ? 'connecting'
        : 'disconnected'
  );

  const locked = $derived(status.state === 'connected' || status.state === 'connecting');

  onMount(() => {
    let unlisten: (() => void) | undefined;
    getStatus()
      .then((s) => (status = s))
      .catch(() => {});
    onStatus((s) => (status = s)).then((f) => (unlisten = f));
    return () => unlisten?.();
  });

  async function toggle() {
    if (busy) return;
    busy = true;
    try {
      if (status.state === 'connected') {
        status = await disconnect();
      } else {
        status = { ...status, state: 'connecting' };
        status = await connect({ kind: upstreamKind });
      }
    } catch (e) {
      status = { state: 'error', error: String(e) };
    } finally {
      busy = false;
    }
  }
</script>

<main>
  <h1>voindns</h1>

  <ConnectButton state={btnState} onclick={toggle} />

  <label class="upstream">
    DNS
    <select bind:value={upstreamKind} disabled={locked}>
      <option value="voindns">voindns</option>
      <option value="cloudflare">Cloudflare</option>
      <option value="google">Google</option>
      <option value="quad9">Quad9</option>
    </select>
  </label>

  {#if status.state === 'connected' && status.listen}
    <p class="meta">{status.listen} → {status.upstream}</p>
  {:else if status.state === 'error' && status.error}
    <p class="err">{status.error}</p>
  {:else}
    <p class="meta">&nbsp;</p>
  {/if}
</main>

<style>
  main {
    height: 100vh;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 28px;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--apricot);
  }
  .upstream {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--muted);
  }
  select {
    background: #2a2a30;
    color: var(--fg);
    border: 1px solid #3a3a42;
    border-radius: 8px;
    padding: 6px 10px;
    font: inherit;
  }
  select:disabled {
    opacity: 0.5;
  }
  .meta {
    margin: 0;
    font-size: 12px;
    color: var(--muted);
    min-height: 1em;
  }
  .err {
    margin: 0;
    font-size: 12px;
    color: var(--err);
    max-width: 300px;
    text-align: center;
  }
</style>
