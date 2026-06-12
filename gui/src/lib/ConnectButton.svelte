<script lang="ts">
  // Amnezia-faithful circular connect button (see ConnectButton.qml).
  type State = 'disconnected' | 'connecting' | 'connected';

  let {
    state = 'disconnected',
    onclick
  }: { state?: State; onclick?: () => void } = $props();

  const RING: Record<State, string> = {
    disconnected: '#D7D8DB',
    connecting: '#261E1A',
    connected: '#FBB26A'
  };
  const GLOW: Record<State, string> = {
    disconnected: 'transparent',
    connecting: 'transparent',
    connected: 'rgba(251, 178, 106, 0.6)'
  };
  const LABEL: Record<State, string> = {
    disconnected: 'Connect',
    connecting: 'Connecting…',
    connected: 'Disconnect'
  };
</script>

<button
  class="cbtn state-{state}"
  onclick={onclick}
  aria-label={LABEL[state]}
  style="--ring:{RING[state]}; --glow:{GLOW[state]}"
>
  <svg viewBox="0 0 190 190">
    <circle cx="95" cy="95" r="93" fill="none" stroke="var(--ring)" stroke-width="3" />
    {#if state === 'connecting'}
      <circle
        class="spin"
        cx="95"
        cy="95"
        r="93"
        fill="none"
        stroke="#D7D8DB"
        stroke-width="3"
        stroke-dasharray="292 292"
        stroke-linecap="round"
      />
    {/if}
  </svg>
  <span class="label">{LABEL[state]}</span>
</button>

<style>
  .cbtn {
    position: relative;
    width: 190px;
    height: 190px;
    border: 0;
    background: transparent;
    border-radius: 50%;
    cursor: pointer;
    display: grid;
    place-items: center;
    filter: drop-shadow(0 0 10px var(--glow));
    transition: filter 0.3s;
  }
  .cbtn svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .label {
    font-weight: 700;
    font-size: 20px;
    color: var(--ring);
    transition: color 0.3s;
  }
  .spin {
    transform-origin: 95px 95px;
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
