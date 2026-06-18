# VoidNS Client — desktop GUI (Tauri + Svelte)

Десктоп-клиент VoidNS под Linux/Windows/macOS. Стек: **Rust + Tauri v2 + Svelte 5 + TS + Vite**.

UI — нативный Svelte 5 порт дизайн-макета «DNS Client Interface»: карточка 350×500
с анимированной эмблемой-ядром, кнопкой Connect/Disconnect, выбором провайдера,
переключателем протокола (DoH активен, DoT — coming soon) и токен-гейтом для
приватного резолвера VoidNS. Окно frameless (`decorations:false` + `transparent`),
управление окном — через титлбар клиента (drag-region + кнопки свернуть/закрыть).

### Подключение (split-privilege, как у AmneziaVPN)

GUI **непривилегированный**. Привилегированную работу (DoH-прокси на `:53` +
смена системного DNS) делает фоновый root-сервис `voidns-service`, который
ставится **один раз** через systemd-юнит. GUI лишь шлёт ему команды
(`connect` / `disconnect` / `get_status`) по локальному сокету
`/run/voidns/control.sock` (chmod 0666 → доступен пользователю, как
`WorldAccessOption` у Amnezia) и слушает живой статус событием `voidns://status`.
Поэтому GUI запускается **без рута** — ровно как клиент Amnezia.

AdGuard/Mullvad/NextDNS уходят как `UpstreamSel::Custom { ip, hostname, path }`,
VoidNS — как `UpstreamSel::Voidns` (токен-гейт — пока UI-гейтинг).

**Запуск сервиса (нужен root для `:53` и смены DNS):**

```bash
cargo build --release -p voidns-service
sudo ./target/release/voidns-service run     # фоновый root-демон
```

После этого запускай GUI **без рута**:

```bash
cd crates/gui && ./run.sh
```

Если сервис не запущен, кнопка Connect покажет **NO SERVICE** с подсказкой; пока
он крутится непривилегированно — **NEEDS ROOT**.

### Провайдер «Dev»

Только в **debug-сборке** (`is_dev()` → `cfg!(debug_assertions)`) внизу списка
появляется провайдер **Dev** — он ничего не подключает, а просто имитирует
состояние таймерами (как в макете). В release-сборке его нет. Вне Tauri
(vite / браузер-превью) бэкенда нет, поэтому имитируются все провайдеры — чтобы
макет оставался живым в браузере.

## Требования (Linux)

- Node 18+ и npm
- Rust (stable) + cargo
- Системные зависимости WebKitGTK:
  - Arch: `sudo pacman -S webkit2gtk-4.1 base-devel`
  - Debian/Ubuntu: `sudo apt install libwebkit2gtk-4.1-dev build-essential libssl-dev libayatana-appindicator3-dev librsvg2-dev`

## Запуск (dev)

```bash
cd crates/gui
npm install
npm run tauri dev
```

Откроется нативное окно с интерфейсом клиента.

> ⚠️ **Не запускай голый бинарь `src-tauri/target/debug/voidns-gui` напрямую** —
> в dev-сборке он грузит фронтенд с dev-сервера `localhost:1420` и без запущенного
> vite покажет «Could not connect to localhost». Запускай через `npm run tauri dev`
> (поднимает vite + окно) либо собери standalone (ниже).

## Standalone-бинарь (без dev-сервера)

```bash
npm run tauri build -- --no-bundle
./src-tauri/target/release/voidns-gui
```

Упаковка в инсталлеры (`bundle`) отключена — собирается только сам бинарь GUI.

## Структура

```
crates/gui/
├── index.html              # точка входа Vite (монтирует Svelte)
├── src/
│   ├── main.ts             # mount(App)
│   ├── app.css             # шрифты, keyframes, прозрачный фон окна
│   ├── lib/backend.ts      # обёртка над Tauri-командами (connect/disconnect/…)
│   └── App.svelte          # нативный UI клиента (порт DNS Client Interface)
└── src-tauri/              # Rust-ядро (Tauri v2)
    ├── src/lib.rs          # команды (one_shot к сервису) + трей + подписка на статус
    ├── src/ipc.rs          # IPC-клиент к voidns-service (фрейминг = voidns-core::ipc)
    ├── icons/tray.png      # иконка трея (favicon сайта frontend)
    └── tauri.conf.json     # frameless transparent окно 420×580
```

Привилегированный сервис: `crates/voidns-service` (запускает
`voidns-core::ipc::serve` с `Controller`) — запускается как root-демон вручную.
