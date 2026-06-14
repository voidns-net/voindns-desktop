<script lang="ts">
  // VoidNS Client — нативный Svelte 5 порт дизайн-макета "DNS Client Interface".
  //
  // Реальные провайдеры (Cloudflare/Google/Quad9/AdGuard/Mullvad/NextDNS и
  // VoidNS) подключаются по-настоящему: команды уходят в привилегированный
  // voidns-service по IPC, тот поднимает DoH-прокси и перенаправляет системный
  // DNS (voidns-core). Провайдер "Dev" виден только в debug-сборке и просто
  // имитирует подключение таймерами, ничего не трогая. Вне Tauri (vite /
  // браузер-превью) бэкенда нет — всё имитируется, чтобы макет оставался живым.
  //
  // Окно frameless: управление окном — через титлбар клиента (drag-region + кнопки).

  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import * as backend from "./lib/backend";
  import type { Status as SvcStatus, UpstreamSel } from "./lib/backend";

  type Status = "disconnected" | "connecting" | "connected" | "error";

  interface Provider {
    id: string;
    name: string;
    doh: string;
    dot: string;
    ip: string;
  }

  const accent = "#00c8ff";
  const danger = "#ff5d6c";

  // Реальные апстримы. AdGuard/Mullvad/NextDNS уходят как UpstreamSel::Custom.
  const BASE_PROVIDERS: Provider[] = [
    { id: "voidns",     name: "VoidNS",      doh: "dns.voidns.net",      dot: "dns.voidns.net",                  ip: "Private" },
    { id: "cloudflare", name: "Cloudflare",  doh: "cloudflare-dns.com",  dot: "1dot1dot1dot1.cloudflare-dns.com", ip: "1.1.1.1" },
    { id: "google",     name: "Google",      doh: "dns.google",          dot: "dns.google",                      ip: "8.8.8.8" },
    { id: "quad9",      name: "Quad9",       doh: "dns.quad9.net",       dot: "dns.quad9.net",                   ip: "9.9.9.9" },
    { id: "adguard",    name: "AdGuard",     doh: "dns.adguard-dns.com", dot: "dns.adguard-dns.com",             ip: "94.140.14.14" },
    { id: "mullvad",    name: "Mullvad",     doh: "dns.mullvad.net",     dot: "dns.mullvad.net",                 ip: "194.242.2.2" },
    { id: "nextdns",    name: "NextDNS",     doh: "dns.nextdns.io",      dot: "dns.nextdns.io",                  ip: "45.90.28.0" },
  ];

  // Имитация — только debug. В рантайме (release) этого провайдера нет.
  const DEV_PROVIDER: Provider = {
    id: "dev", name: "Dev", doh: "simulated", dot: "simulated", ip: "mock",
  };

  // Реальный бэкенд доступен только внутри Tauri; "dev" всегда имитируется.
  const tauriAvailable = backend.isTauri();
  const usesBackend = (id: string) => tauriAvailable && id !== "dev";

  function upstreamFor(id: string): UpstreamSel {
    switch (id) {
      case "voidns": return { kind: "voidns" };
      case "cloudflare": return { kind: "cloudflare" };
      case "google": return { kind: "google" };
      case "quad9": return { kind: "quad9" };
      case "adguard": return { kind: "custom", ip: "94.140.14.14", hostname: "dns.adguard-dns.com", path: "/dns-query" };
      case "mullvad": return { kind: "custom", ip: "194.242.2.2", hostname: "dns.mullvad.net", path: "/dns-query" };
      case "nextdns": return { kind: "custom", ip: "45.90.28.0", hostname: "dns.nextdns.io", path: "/dns-query" };
      default: return { kind: "cloudflare" };
    }
  }

  // --- reactive state ---------------------------------------------------------
  let status = $state<Status>("disconnected");
  const protocol = "DoH"; // DoT — coming soon (переключатель пока неактивен)
  let providerId = $state("cloudflare");
  let open = $state(false);
  let elapsed = $state(0);
  let token = $state("");
  let tokenInput = $state("");
  let showTokenGate = $state(true);
  let devVisible = $state(false);
  let connError = $state<string | null>(null); // short label shown under the core
  let errorDetail = $state<string | null>(null); // full backend message (tooltip)

  // Превратить сырую ошибку в короткий понятный код для статуса.
  function humanizeError(msg: string | null | undefined): string {
    const m = (msg ?? "").toLowerCase();
    if (/unreachable|refused|no such file|not found|enoent|cannot connect/.test(m)) return "NO SERVICE";
    if (/authoriz|denied|permission|privileg|not permitted|eperm/.test(m)) return "NEEDS ROOT";
    if (/address.*in use|already in use|eaddrinuse|bind/.test(m)) return "PORT IN USE";
    return "ERROR";
  }

  const providers = $derived<Provider[]>(
    devVisible ? [...BASE_PROVIDERS, DEV_PROVIDER] : BASE_PROVIDERS,
  );

  let connectTimer: ReturnType<typeof setTimeout> | undefined;
  let tickTimer: ReturnType<typeof setInterval> | undefined;

  function clearTimers() {
    clearTimeout(connectTimer);
    clearInterval(tickTimer);
  }
  function startTick() {
    clearInterval(tickTimer);
    tickTimer = setInterval(() => (elapsed += 1), 1000);
  }

  // --- simulation (Dev provider / вне Tauri) ----------------------------------
  function simConnect(delay = 1500) {
    clearTimers();
    status = "connecting";
    elapsed = 0;
    connectTimer = setTimeout(() => {
      status = "connected";
      elapsed = 0;
      startTick();
    }, delay);
  }
  function simDisconnect() {
    clearTimers();
    status = "disconnected";
    elapsed = 0;
  }

  // --- real backend (voidns-service over IPC) ---------------------------------
  function applyStatus(s: SvcStatus) {
    switch (s.state) {
      case "connected":
        if (status !== "connected") {
          elapsed = 0;
          startTick();
        }
        status = "connected";
        connError = null;
        break;
      case "connecting":
        status = "connecting";
        break;
      case "error":
        clearTimers();
        status = "error";
        if (s.error) {
          errorDetail = s.error;
          connError = humanizeError(s.error);
          console.error("[voidns] connect error:", s.error);
        }
        break;
      default:
        clearTimers();
        status = "disconnected";
        elapsed = 0;
    }
  }

  async function realConnect() {
    clearTimers();
    status = "connecting";
    elapsed = 0;
    connError = null;
    errorDetail = null;
    try {
      applyStatus(await backend.connect(upstreamFor(providerId)));
    } catch (e) {
      errorDetail = String(e);
      connError = humanizeError(String(e));
      status = "error";
      console.error("[voidns] connect failed:", e);
    }
  }
  async function realDisconnect() {
    clearTimers();
    try {
      applyStatus(await backend.disconnect());
    } catch (e) {
      status = "disconnected";
      elapsed = 0;
      console.error("[voidns] disconnect failed:", e);
    }
  }

  // Смена провайдера на лету: разорвать старое подключение, поднять новое.
  async function reconnectFor(prevWasBackend: boolean) {
    if (status === "disconnected") return;
    clearTimers();
    if (prevWasBackend) {
      try {
        await backend.disconnect();
      } catch (e) {
        console.error("[voidns] disconnect failed:", e);
      }
    }
    if (usesBackend(providerId)) {
      status = "connecting";
      elapsed = 0;
      connError = null;
      try {
        applyStatus(await backend.connect(upstreamFor(providerId)));
      } catch (e) {
        connError = "CONNECT FAILED";
        status = "error";
        console.error("[voidns] reconnect failed:", e);
      }
    } else {
      simConnect(900); // Dev mock
    }
  }

  // --- actions ----------------------------------------------------------------
  function toggle() {
    if (status === "disconnected" || status === "error") {
      if (providerId === "voidns" && !token) {
        clearTimers();
        status = "error";
        connError = null;
        open = false;
        showTokenGate = true;
        return;
      }
      connError = null;
      if (usesBackend(providerId)) realConnect();
      else simConnect();
    } else {
      if (usesBackend(providerId)) realDisconnect();
      else simDisconnect();
    }
  }

  function selectProvider(id: string) {
    const changed = id !== providerId;
    const wasConnected = status === "connected" || status === "connecting";
    const prevWasBackend = usesBackend(providerId);
    providerId = id;
    open = false;
    if (!changed) return;
    if (id === "voidns" && !token) {
      clearTimers();
      status = "error";
      connError = null;
      showTokenGate = true;
      return;
    }
    if (wasConnected) reconnectFor(prevWasBackend);
  }

  const toggleOpen = () => (open = !open);

  function activateToken() {
    const t = tokenInput.trim();
    if (!t) return;
    token = t;
    showTokenGate = false;
    providerId = "voidns";
    if (status === "error") {
      status = "disconnected";
      connError = null;
    }
  }
  const skipGate = () => (showTokenGate = false);
  function openGate() {
    open = false;
    showTokenGate = true;
  }

  onMount(() => {
    let unlisten: () => void = () => {};
    (async () => {
      devVisible = await backend.isDevBuild();
      if (tauriAvailable) {
        unlisten = await backend.onStatus((s) => {
          if (providerId === "dev") return; // Dev — локальная имитация
          applyStatus(s);
        });
        try {
          const s = await backend.getStatus();
          if (providerId !== "dev") applyStatus(s);
        } catch {
          // service not up yet — subscription will catch up
        }
      }
    })();
    return () => unlisten();
  });

  // --- window controls --------------------------------------------------------
  async function minimize() {
    try { await getCurrentWindow().minimize(); } catch (e) { console.error(e); }
  }
  async function close() {
    try { await getCurrentWindow().close(); } catch (e) { console.error(e); }
  }

  // --- derived values ---------------------------------------------------------
  const on = $derived(status === "connected");
  const busy = $derived(status === "connecting");
  const err = $derived(status === "error");
  const sel = $derived(providers.find((p) => p.id === providerId) ?? providers[0]);
  const hasToken = $derived(!!token);

  const endpoint = $derived(
    providerId === "dev"
      ? "simulated — no real DNS change"
      : protocol === "DoH"
        ? `https://${sel.doh}/dns-query`
        : `tls://${sel.dot}:853`,
  );
  const elapsedStr = $derived(
    `${String(Math.floor(elapsed / 60)).padStart(2, "0")}:${String(elapsed % 60).padStart(2, "0")}`,
  );

  const statusColor = $derived(
    on ? accent : busy ? "#ffb454" : err ? danger : "rgba(232,234,246,.85)",
  );
  const statusLabel = $derived(
    on
      ? "PROTECTED"
      : busy
        ? "ESTABLISHING"
        : err
          ? (connError ?? "TOKEN REQUIRED")
          : "NOT CONNECTED",
  );
  const actionLabel = $derived(on ? "DISCONNECT" : busy ? "CANCEL" : "CONNECT");

  // emblem animation styles
  const spin = (d: number) =>
    on ? `vn-spin ${d}s linear infinite` : busy ? "vn-spin 2.6s linear infinite" : "none";
  const spinrev = (d: number) =>
    on ? `vn-spinrev ${d}s linear infinite` : busy ? "vn-spinrev 1.5s linear infinite" : "none";
  const grp = (anim: string, op: number) =>
    `transform-box:fill-box;transform-origin:center;animation:${anim};opacity:${op};transition:opacity .6s ease`;

  const ringStyle = $derived(grp(spin(34), on ? 0.95 : busy ? 0.75 : 0.4));
  const ring2Style = $derived(grp(spinrev(22), on ? 0.9 : busy ? 0.7 : 0.38));
  const nodeStyle = $derived(grp(spinrev(15), on ? 0.95 : busy ? 0.75 : 0.32));
  const coreStyle = $derived(
    `transform-box:fill-box;transform-origin:center;animation:${on ? "vn-core 2.6s ease-in-out infinite" : busy ? "vn-core 0.95s ease-in-out infinite" : "none"}`,
  );
  const rimStyle = $derived(`opacity:${on ? 1 : busy ? 0.7 : 0.22};transition:opacity .6s ease`);

  const statusDotStyle = $derived(
    `width:7px;height:7px;border-radius:50%;background:${statusColor};box-shadow:${on || busy || err ? "0 0 9px " + statusColor : "none"};transition:all .4s ease`,
  );
  const statusLabelStyle = $derived(
    `font-family:'Inter',sans-serif;font-size:17px;font-weight:700;letter-spacing:2.5px;color:${statusColor};transition:color .4s ease`,
  );
  const caretStyle = $derived(
    `transition:transform .25s ease;transform:${open ? "rotate(180deg)" : "rotate(0deg)"}`,
  );

  const btnBase =
    "width:100%;padding:13px 0;border-radius:11px;font-family:'Inter',sans-serif;font-size:14.5px;font-weight:700;letter-spacing:1px;cursor:pointer;transition:all .3s ease;border:1px solid transparent";
  const buttonStyle = $derived(
    on
      ? `${btnBase};background:rgba(0,200,255,.06);color:${accent};border-color:rgba(0,200,255,.45)`
      : busy
        ? `${btnBase};background:transparent;color:#ffb454;border-color:rgba(255,180,84,.55)`
        : `${btnBase};background:${accent};color:#03030c;box-shadow:0 0 26px rgba(0,200,255,.5)`,
  );

  const seg = (active: boolean) =>
    `flex:1;padding:7px 0;text-align:center;font-family:'JetBrains Mono',monospace;font-size:12.5px;font-weight:700;letter-spacing:.3px;cursor:default;border:none;border-radius:6px;background:${active ? "rgba(0,200,255,.16)" : "transparent"};color:${active ? accent : "rgba(232,234,246,.3)"};box-shadow:${active ? "inset 0 0 0 1px rgba(0,200,255,.4)" : "none"};transition:all .2s ease`;

  const tokenReady = $derived(!!tokenInput.trim());
  const activateStyle = $derived(
    `width:100%;margin-top:16px;padding:12px 0;border-radius:10px;border:1px solid transparent;font-family:'Inter',sans-serif;font-size:13px;font-weight:700;letter-spacing:1.5px;transition:all .25s ease;cursor:${tokenReady ? "pointer" : "not-allowed"};background:${tokenReady ? accent : "rgba(0,200,255,.10)"};color:${tokenReady ? "#03030c" : "rgba(232,234,246,.4)"};box-shadow:${tokenReady ? "0 0 22px rgba(0,200,255,.45)" : "none"}`,
  );

  interface ProviderRow extends Provider {
    host: string;
    selected: boolean;
    lockShow: boolean;
    nameColor: string;
    rowStyle: string;
  }
  const providersView = $derived<ProviderRow[]>(
    providers.map((p) => {
      const lock = p.id === "voidns" && !token;
      return {
        ...p,
        host:
          p.id === "dev"
            ? "simulated — no real DNS"
            : lock
              ? "access token required"
              : protocol === "DoH"
                ? p.doh
                : `${p.dot}:853`,
        selected: p.id === providerId,
        lockShow: lock,
        nameColor: p.id === providerId ? accent : "#e8eaf6",
        rowStyle: `display:flex;align-items:center;gap:10px;padding:10px 12px;cursor:pointer;border-bottom:1px solid rgba(0,200,255,.06);background:${p.id === providerId ? "rgba(0,200,255,.10)" : "transparent"};transition:background .15s ease`,
      };
    }),
  );
</script>

<div class="card">
  <!-- starfield -->
  <span class="star" style="left:26px;top:64px;width:2px;height:2px;background:#fff;opacity:.5;animation:vn-tw 4.2s ease-in-out infinite"></span>
  <span class="star" style="left:300px;top:96px;width:2px;height:2px;background:#9fe8ff;opacity:.5;animation:vn-tw 5.1s ease-in-out .6s infinite"></span>
  <span class="star" style="left:54px;top:300px;width:1.5px;height:1.5px;background:#fff;opacity:.4;animation:vn-tw 3.6s ease-in-out 1.2s infinite"></span>
  <span class="star" style="left:312px;top:330px;width:2px;height:2px;background:#fff;opacity:.45;animation:vn-tw 4.8s ease-in-out .3s infinite"></span>
  <span class="star" style="left:30px;top:430px;width:1.5px;height:1.5px;background:#9fe8ff;opacity:.4;animation:vn-tw 5.4s ease-in-out 1.8s infinite"></span>
  <span class="star" style="left:160px;top:50px;width:1.5px;height:1.5px;background:#fff;opacity:.4;animation:vn-tw 4s ease-in-out 2.2s infinite"></span>

  <!-- titlebar -->
  <div class="titlebar" data-tauri-drag-region>
    <div class="tb-left" data-tauri-drag-region>
      <div style={statusDotStyle}></div>
      <span class="tb-label">DNS PRIVACY</span>
      {#if hasToken}
        <button class="token-chip" onclick={openGate}>TOKEN</button>
      {/if}
    </div>
    {#if on}
      <span class="tb-timer">
        <span class="tb-timer-dot"></span>{elapsedStr}
      </span>
    {/if}
    <div class="tb-controls">
      <button class="winbtn" aria-label="Minimize" onclick={minimize}>
        <span class="win-min"></span>
      </button>
      <button class="winbtn" aria-label="Close" onclick={close}>
        <svg width="11" height="11" viewBox="0 0 12 12"><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>
      </button>
    </div>
  </div>

  <!-- header -->
  <div class="header">
    <div class="brand">VOIDNS</div>
    <div class="brand-sub">CLIENT</div>
  </div>

  <!-- emblem + status -->
  <div class="emblem-wrap">
    <button class="emblem-btn" onclick={toggle} aria-label="Toggle connection">
      <svg width="166" height="166" viewBox="0 0 200 200" style="overflow:visible">
        <defs>
          <radialGradient id="vnCore" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stop-color="#101028"></stop>
            <stop offset="60%" stop-color="#07071a"></stop>
            <stop offset="100%" stop-color="#020209"></stop>
          </radialGradient>
          <radialGradient id="vnRim" cx="50%" cy="50%" r="50%">
            <stop offset="68%" stop-color="#00c8ff" stop-opacity="0"></stop>
            <stop offset="100%" stop-color="#00c8ff" stop-opacity="0.5"></stop>
          </radialGradient>
          <filter id="vnGlow" x="-40%" y="-40%" width="180%" height="180%">
            <feGaussianBlur stdDeviation="1.7" result="b"></feGaussianBlur>
            <feMerge><feMergeNode in="b"></feMergeNode><feMergeNode in="SourceGraphic"></feMergeNode></feMerge>
          </filter>
        </defs>

        <circle cx="100" cy="100" r="82" fill="url(#vnCore)"></circle>
        <circle cx="100" cy="100" r="82" fill="url(#vnRim)" style={rimStyle}></circle>
        <circle cx="100" cy="100" r="82" fill="none" stroke="#00c8ff" stroke-width="1.1" opacity="0.5"></circle>

        {#if on}
          <circle cx="100" cy="100" r="82" fill="none" stroke="#00c8ff" stroke-width="1" style="transform-box:fill-box;transform-origin:center;animation:vn-ping 3.4s ease-out infinite"></circle>
          <circle cx="100" cy="100" r="82" fill="none" stroke="#00c8ff" stroke-width="1" style="transform-box:fill-box;transform-origin:center;animation:vn-ping 3.4s ease-out 1.7s infinite"></circle>
        {/if}

        <g filter="url(#vnGlow)" style={ringStyle}>
          <circle cx="100" cy="100" r="66" fill="none" stroke="#00c8ff" stroke-width="0.8" stroke-dasharray="2 8" opacity="0.5"></circle>
          <circle cx="100" cy="100" r="58" fill="none" stroke="#00c8ff" stroke-width="1" stroke-dasharray="46 26" opacity="0.6"></circle>
        </g>

        <g filter="url(#vnGlow)" style={ring2Style}>
          <circle cx="100" cy="100" r="42" fill="none" stroke="#00c8ff" stroke-width="1.1" stroke-dasharray="30 18" opacity="0.7"></circle>
          <circle cx="100" cy="100" r="28" fill="none" stroke="#00c8ff" stroke-width="1.3" opacity="0.75"></circle>
        </g>

        <g filter="url(#vnGlow)" style={nodeStyle}>
          <g stroke="#00c8ff" stroke-width="0.8" opacity="0.5">
            <line x1="100" y1="100" x2="100" y2="42"></line>
            <line x1="100" y1="100" x2="150.2" y2="129"></line>
            <line x1="100" y1="100" x2="49.8" y2="129"></line>
          </g>
          <g fill="#00c8ff">
            <circle cx="100" cy="42" r="2.6"></circle>
            <circle cx="150.2" cy="129" r="2.6"></circle>
            <circle cx="49.8" cy="129" r="2.6"></circle>
          </g>
        </g>

        <g filter="url(#vnGlow)" style={coreStyle}>
          <circle cx="100" cy="100" r="7" fill="#00c8ff" opacity="0.95"></circle>
          <circle cx="100" cy="100" r="3" fill="#ffffff"></circle>
        </g>
      </svg>
    </button>

    <div style={statusLabelStyle} title={err && connError ? (errorDetail ?? "") : ""}>{statusLabel}</div>
    {#if err && (connError === "NO SERVICE" || connError === "NEEDS ROOT")}
      <div class="status-hint">
        {connError === "NO SERVICE" ? "service not running" : "service needs root"} —
        <span>sudo installers/linux/install-dev.sh</span>
      </div>
    {:else if err && connError && connError !== "TOKEN REQUIRED"}
      <div class="status-hint" title={errorDetail ?? ""}>{errorDetail ?? connError}</div>
    {/if}
  </div>

  <!-- controls -->
  <div class="controls">
    <button onclick={toggle} style={buttonStyle}>{actionLabel}</button>

    <div class="row">
      <div class="proto-wrap">
        <div class="proto" title="Protocol switching coming soon">
          <div style={seg(protocol === "DoH")}>DoH</div>
          <div style={seg(false)}>DoT</div>
        </div>
      </div>

      <div class="select-wrap">
        <button class="select-btn" onclick={toggleOpen}>
          <span class="select-text">
            <span class="select-name">{sel.name}</span>
            <span class="select-ip">{sel.ip}</span>
          </span>
          <svg width="11" height="11" viewBox="0 0 12 12" fill="none" style={caretStyle + ";flex:none"}><path d="M2 4l4 4 4-4" stroke="#00c8ff" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"></path></svg>
        </button>

        {#if open}
          <div class="dropdown">
            {#each providersView as p (p.id)}
              <button class="prow" style={p.rowStyle} onclick={() => selectProvider(p.id)}>
                <span class="prow-text">
                  <span class="prow-name" style="color:{p.nameColor}">{p.name}</span>
                  <span class="prow-host">{p.host}</span>
                </span>
                {#if p.lockShow}
                  <svg width="11" height="12" viewBox="0 0 10 12" fill="none" style="flex:none"><rect x="1.5" y="5" width="7" height="6" rx="1.2" stroke="#ffb454" stroke-width="1" opacity=".85"></rect><path d="M3 5V3.5a2 2 0 014 0V5" stroke="#ffb454" stroke-width="1" opacity=".85"></path></svg>
                {/if}
                {#if p.selected}
                  <span class="prow-sel"></span>
                {/if}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    </div>

    <div class="endpoint">
      <svg width="10" height="12" viewBox="0 0 10 12" fill="none" style="flex:none"><rect x="1.5" y="5" width="7" height="6" rx="1.2" stroke="#00c8ff" stroke-width="1" opacity=".7"></rect><path d="M3 5V3.5a2 2 0 014 0V5" stroke="#00c8ff" stroke-width="1" opacity=".7"></path></svg>
      <span class="endpoint-text">{endpoint}</span>
    </div>
  </div>

  <!-- token gate -->
  {#if showTokenGate}
    <div class="gate">
      <div class="gate-card">
        <div class="gate-icon">
          <svg width="46" height="50" viewBox="0 0 46 50" fill="none">
            <circle cx="23" cy="23" r="20" stroke="#00c8ff" stroke-width="1" opacity=".28"></circle>
            <circle cx="23" cy="23" r="13" stroke="#00c8ff" stroke-width="0.8" opacity=".22"></circle>
            <rect x="15" y="22" width="16" height="13" rx="2.6" stroke="#00c8ff" stroke-width="1.5"></rect>
            <path d="M18 22v-3.4a5 5 0 0110 0V22" stroke="#00c8ff" stroke-width="1.5"></path>
            <circle cx="23" cy="28" r="2" fill="#00c8ff"></circle>
          </svg>
        </div>
        <div class="gate-title">ACCESS TOKEN</div>
        <div class="gate-desc">Enter your VoidNS token to unlock the private resolver. Other providers stay open.</div>
        <input
          class="gate-input"
          bind:value={tokenInput}
          placeholder="voidns_xxxxxxxxxxxx"
          spellcheck="false"
          autocomplete="off"
          onkeydown={(e) => e.key === "Enter" && activateToken()}
        />
        <button style={activateStyle} onclick={activateToken}>ACTIVATE</button>
        <button class="gate-skip" onclick={skipGate}>SKIP FOR NOW</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .card {
    position: relative;
    width: 350px;
    height: 540px;
    border-radius: 18px;
    overflow: hidden;
    background: radial-gradient(circle at 50% 34%, #0e0e22 0%, #08081a 52%, #030309 100%);
    border: 1px solid rgba(0, 200, 255, 0.18);
    box-shadow: none;
    display: flex;
    flex-direction: column;
    color: #e8eaf6;
  }

  .star {
    position: absolute;
    border-radius: 50%;
    pointer-events: none;
  }

  /* titlebar */
  .titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px;
    height: 36px;
    flex: none;
    border-bottom: 1px solid rgba(0, 200, 255, 0.08);
    position: relative;
    z-index: 2;
  }
  .tb-left { display: flex; align-items: center; gap: 8px; }
  .tb-label {
    font-family: "JetBrains Mono", monospace;
    font-size: 10.5px;
    letter-spacing: 2.5px;
    color: rgba(232, 234, 246, 0.62);
  }
  .token-chip {
    font-family: "JetBrains Mono", monospace;
    font-size: 9px;
    letter-spacing: 1.5px;
    color: rgba(0, 200, 255, 0.75);
    border: 1px solid rgba(0, 200, 255, 0.3);
    border-radius: 5px;
    padding: 1px 5px;
    cursor: pointer;
    background: transparent;
  }
  .tb-timer {
    font-family: "JetBrains Mono", monospace;
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 13px;
    font-weight: 500;
    letter-spacing: 1.5px;
    color: #00c8ff;
    text-shadow: 0 0 10px rgba(0, 200, 255, 0.5);
  }
  .tb-timer-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #00c8ff;
    box-shadow: 0 0 9px #00c8ff;
  }
  .tb-controls {
    display: flex;
    align-items: center;
    gap: 10px;
    color: rgba(232, 234, 246, 0.35);
  }
  .winbtn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    transition: color 0.2s ease;
  }
  .winbtn:hover { color: rgba(232, 234, 246, 0.9); }
  .win-min { width: 11px; height: 1px; background: currentColor; }

  /* header */
  .header { text-align: center; padding: 15px 0 6px; flex: none; position: relative; z-index: 2; }
  .brand {
    font-family: "JetBrains Mono", monospace;
    font-size: 36px;
    font-weight: 800;
    letter-spacing: 7px;
    color: #eef0ff;
    text-shadow: 0 0 18px rgba(0, 200, 255, 0.55), 0 0 2px rgba(0, 200, 255, 0.8);
    line-height: 1;
  }
  .brand-sub {
    font-family: "JetBrains Mono", monospace;
    margin-top: 7px;
    font-size: 11.5px;
    font-weight: 500;
    letter-spacing: 8px;
    color: rgba(0, 200, 255, 0.7);
  }

  /* emblem */
  .emblem-wrap {
    flex: 1;
    min-height: 0; /* allow the middle to absorb the error hint instead of
                      pushing the controls/endpoint past the card's clipped edge */
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    position: relative;
    z-index: 2;
    gap: 6px;
  }
  .status-hint {
    max-width: 280px;
    margin-top: 2px;
    padding: 0 18px;
    font-family: "JetBrains Mono", monospace;
    font-size: 10px;
    letter-spacing: 0.3px;
    line-height: 1.4;
    text-align: center;
    color: rgba(255, 93, 108, 0.72);
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .status-hint span {
    color: #ffb454;
  }
  .emblem-btn {
    border: none;
    background: transparent;
    padding: 0;
    cursor: pointer;
    line-height: 0;
    border-radius: 50%;
    transition: transform 0.3s ease;
  }
  .emblem-btn:hover { transform: scale(1.04); }

  /* controls */
  .controls { flex: none; padding: 16px 22px 22px; position: relative; z-index: 3; }
  .row { display: flex; gap: 18px; margin-top: 16px; }
  .proto-wrap { flex: none; width: 88px; }
  .proto {
    display: flex;
    gap: 4px;
    background: rgba(0, 200, 255, 0.04);
    border: 1px solid rgba(0, 200, 255, 0.12);
    border-radius: 9px;
    padding: 3px;
    pointer-events: none;
    opacity: 0.85;
  }
  .select-wrap { flex: 1; position: relative; }
  .select-btn {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    background: rgba(0, 200, 255, 0.04);
    border: 1px solid rgba(0, 200, 255, 0.18);
    border-radius: 9px;
    padding: 8px 11px;
    cursor: pointer;
    color: #e8eaf6;
    font-family: inherit;
    transition: border-color 0.2s ease;
  }
  .select-btn:hover { border-color: rgba(0, 200, 255, 0.4); }
  .select-text { display: flex; flex-direction: column; align-items: flex-start; gap: 2px; min-width: 0; }
  .select-name { font-size: 14.5px; font-weight: 700; letter-spacing: 0.5px; }
  .select-ip { font-family: "JetBrains Mono", monospace; font-size: 11px; color: rgba(232, 234, 246, 0.62); }

  .dropdown {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 6px);
    background: rgba(8, 8, 22, 0.97);
    border: 1px solid rgba(0, 200, 255, 0.25);
    border-radius: 11px;
    overflow: hidden auto;
    box-shadow: 0 -10px 40px rgba(0, 0, 0, 0.6), 0 0 30px rgba(0, 200, 255, 0.12);
    backdrop-filter: blur(8px);
    max-height: 268px;
  }
  .prow { width: 100%; border: none; background: transparent; text-align: left; font-family: inherit; }
  .prow-text { flex: 1; min-width: 0; display: block; }
  .prow-name { display: block; font-size: 13.5px; font-weight: 700; letter-spacing: 0.5px; }
  .prow-host {
    display: block;
    font-family: "JetBrains Mono", monospace;
    font-size: 10.5px;
    color: rgba(232, 234, 246, 0.55);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .prow-sel { flex: none; width: 6px; height: 6px; border-radius: 50%; background: #00c8ff; box-shadow: 0 0 8px #00c8ff; }

  .endpoint {
    font-family: "JetBrains Mono", monospace;
    margin-top: 14px;
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 11.5px;
    color: rgba(232, 234, 246, 0.6);
    white-space: nowrap;
    overflow: hidden;
  }
  .endpoint-text { overflow: hidden; text-overflow: ellipsis; }

  /* token gate */
  .gate {
    position: absolute;
    inset: 0;
    z-index: 20;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 28px;
    background: rgba(4, 4, 14, 0.88);
    backdrop-filter: blur(7px);
  }
  .gate-card {
    width: 100%;
    border: 1px solid rgba(0, 200, 255, 0.22);
    border-radius: 16px;
    background: radial-gradient(circle at 50% 0%, #0f0f26 0%, #07071a 70%, #050511 100%);
    box-shadow: 0 0 44px rgba(0, 200, 255, 0.16), 0 18px 50px rgba(0, 0, 0, 0.6);
    padding: 26px 22px;
    text-align: center;
  }
  .gate-icon { display: flex; justify-content: center; margin-bottom: 14px; }
  .gate-title {
    font-family: "JetBrains Mono", monospace;
    font-size: 15.5px;
    font-weight: 800;
    letter-spacing: 3px;
    color: #eef0ff;
  }
  .gate-desc {
    margin: 9px auto 0;
    font-size: 11.5px;
    line-height: 1.6;
    letter-spacing: 0.3px;
    color: rgba(232, 234, 246, 0.52);
    text-wrap: pretty;
    max-width: 256px;
  }
  .gate-input {
    width: 100%;
    margin-top: 18px;
    padding: 12px 13px;
    background: rgba(0, 200, 255, 0.05);
    border: 1px solid rgba(0, 200, 255, 0.25);
    border-radius: 9px;
    color: #e8eaf6;
    font-family: "JetBrains Mono", monospace;
    font-size: 13px;
    letter-spacing: 1px;
    outline: none;
  }
  .gate-input:focus { border-color: rgba(0, 200, 255, 0.55); }
  .gate-skip {
    margin-top: 14px;
    font-size: 11px;
    letter-spacing: 2px;
    color: rgba(232, 234, 246, 0.42);
    cursor: pointer;
    background: transparent;
    border: none;
    transition: color 0.2s ease;
  }
  .gate-skip:hover { color: rgba(232, 234, 246, 0.75); }
</style>
