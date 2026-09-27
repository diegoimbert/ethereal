# Ethereal

An open-source (GPL-3.0-or-later) Ableton-style DAW. Rust engine, React/TypeScript UI.
The UI runs unchanged in a Tauri desktop app and in the browser; the engine core compiles
natively (macOS/Windows/Linux) and to WebAssembly.

- Architecture and locked decisions: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Contracts review guide (protocol, model, engine traits, transport): [docs/CONTRACTS.md](docs/CONTRACTS.md)
- How the v0.1 build is parallelized: [docs/ORCHESTRATION.md](docs/ORCHESTRATION.md)

> Status: scaffold. Contracts are in place; most logic is `todo!()` and is being filled in
> by parallel work streams (see ORCHESTRATION.md).

## Toolchain setup

| Tool | Version | Install |
|---|---|---|
| Rust | stable (pinned by `rust-toolchain.toml`) | [rustup](https://rustup.rs); the toolchain file installs `rustfmt`, `clippy` and the `wasm32-unknown-unknown` target automatically on first `cargo` run (or `rustup target add wasm32-unknown-unknown`). |
| wasm-bindgen CLI | must match the `wasm-bindgen` crate version in `Cargo.lock` | `cargo install wasm-bindgen-cli --version <x.y.z>`. Only needed to build the browser engine (wasm-host work), not for checks. |
| Node.js | >= 20 (CI uses 24) | [nodejs.org](https://nodejs.org) or `fnm`/`nvm` |
| pnpm | as pinned in `package.json` `packageManager` | `corepack enable` (or `npm i -g pnpm`) |
| just | any recent | `brew install just` / `cargo install just` / [just.systems](https://just.systems) |
| Tauri CLI | v2 | Installed as a dev dependency of `apps/desktop` (`pnpm tauri ...`); no global install needed. |
| C/C++ toolchain | | macOS: Xcode Command Line Tools (`xcode-select --install`). Windows: MSVC Build Tools + WebView2 (preinstalled on Win 11). Linux: `build-essential` plus Tauri/ALSA deps: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libssl-dev pkg-config`. Needed for Tauri, cpal, and Signalsmith Stretch (C++). |
| Python | >= 3.11 | Only for `scripts/check-ownership.py` (uses `tomllib`). |

```sh
pnpm install
just check-all   # fmt, clippy, wasm32 check, tsc, eslint
just test-all    # cargo test + vitest
```

## Everyday commands (`just --list`)

| Command | What it does |
|---|---|
| `just dev-ui` | Standalone UI against the in-memory `MockTransport` (no Rust needed). |
| `just dev-web` | Browser host (`apps/web`, COOP/COEP headers for SharedArrayBuffer). |
| `just dev-desktop` | Tauri desktop app. `just dev-desktop-headless` = same with the null audio backend. |
| `just dev-server` | Headless engine over WebSocket (`ether-server`) for the web UI's **Remote** button (see below). |
| `just gen-types` | Regenerate `ui/src/generated/` (TS types) from `crates/ether-protocol`. Commit the result; CI fails if stale. |
| `just check-wasm` | `cargo check --target wasm32-unknown-unknown` for the engine-side crates. |
| `just check-all` / `just test-all` | Everything CI runs. |
| `just check-ownership` | Checks that a `node/<id>` branch only touches its owned paths (`.github/ownership.toml`). |
| `just dev-port` | Prints this checkout's instance id and ports. |

## Layout

```
crates/
  ether-protocol        commands/events/replies, TS type export      (native + wasm)
  ether-model           document types, IDs, ops, undo, .ether format (native + wasm)
  ether-core            RT engine: graph, scheduler, transport, mixer (native + wasm)
  ether-devices         synth, sampler, compressor, delay            (native + wasm)
  ether-media           decode, resample, peaks                      (native + wasm)
  ether-controller      commands -> ops -> patches; model -> engine  (native + wasm)
  ether-stretch         Stretcher trait + Signalsmith                (trait wasm, impl native)
  ether-clap            CLAP hosting (clack)                         (native)
  ether-plugin-scanner  out-of-process scanner binary                (native)
  ether-sandbox         out-of-process plugin sandbox                (native)
  ether-native          cpal/null audio host, threads                (native)
  ether-wasm            wasm-bindgen worker + worklet entry points   (wasm)
apps/desktop            Tauri v2 shell (apps/desktop/src-tauri = crate ether-desktop)
apps/web                Vite host for the browser build
ui/                     React + TS app (@ethereal/ui)
  src/generated/        GENERATED protocol types (just gen-types)
  src/transport/        EngineTransport + MockTransport (+ tauri/, wasm/)
  src/state/            UI mirror of the project (zustand), playhead/meter store
  src/kit/              primitives: theme, Button, Knob, Fader, Panel, Meter
  src/timeline/         time<->pixel, zoom/scroll, grid, selection
  src/features/<name>/  one folder per feature
scripts/                dev-env.mjs (instance/ports), check-ownership.py
```

## Running multiple dev instances

Several checkouts (git worktrees, parallel agents) can run dev servers, the desktop app
and tests at the same time without clashing. Always go through the `just` recipes, and
never hardcode ports or paths.

- **Instance id.** `ETHER_INSTANCE` names the instance. It defaults to the checkout
  (worktree) directory name, sanitized to `[A-Za-z0-9_-]`. Everything below derives from it
  (`scripts/dev-env.mjs` is the single source of truth).
- **Ports.** No fixed ports. The base port is `20000 + fnv1a(instance) % 10000`, or
  `ETHER_DEV_PORT` when set. Offsets: `+0` UI/web dev server (and the Tauri `devUrl`),
  `+1` vite preview, `+2` Playwright web server, `+3` reserved (collab server), `+4` remote engine server (`ether-server`). Vite runs
  with `strictPort`, so a collision fails loudly instead of silently moving. If two
  instance names ever hash to the same port, set `ETHER_DEV_PORT` for one of them. Other
  servers use the same scheme, or port 0.
- **Desktop identity and data.** `just dev-desktop` overrides the Tauri identifier to
  `dev.ethereal.<instance>` (separate WebView storage) and injects `devUrl` through
  `tauri dev --config` (it's not hardcoded in `tauri.conf.json`). In debug builds all app
  data goes under `<data_dir>/ethereal-dev/<instance>/`: `config/`, `logs/`, `cache/`,
  `plugin-db/`, `autosave/`, `tmp/` and `projects/` (the dev `projects_root` of the
  engine-side project store). There's no single-instance behavior.
- **Headless audio.** `ETHER_AUDIO=null` selects the null backend. The engine runs on a
  timer thread with no sound device, so nothing contends for the audio device.
  `ETHER_AUDIO=offline` renders as fast as possible. Engine tests render offline and never
  need a device. CI sets `ETHER_AUDIO=null`.
- **IPC names.** Every globally named OS object (shared memory, semaphores, sockets or
  pipes of the plugin sandbox, scanner temp files) is named with
  `ether_core::plugin::ipc_name(instance, pid, purpose)`, so it includes both the instance
  id and the pid.
- **Cargo.** Each worktree keeps its own `target/` directory, which is the default. Don't
  set a shared `CARGO_TARGET_DIR`: the build-directory lock would serialize every agent.

## Remote engine (`ether-server`)

`ether-server` runs the engine headless (the same native host as the desktop app) and
serves the UI over WebSocket, so the browser UI can drive an engine on another machine.

- `just dev-server` starts it on this instance's remote port (base `+4`, see
  `just dev-port`) with the null audio backend and prints its address and token. In the
  web UI (`just dev-web`), click **Remote** in the top bar, enter `ws://127.0.0.1:<port>`
  and the token. Disconnecting returns to the in-browser engine.
- It listens on `127.0.0.1` by default; `--listen <ip>` exposes it to the network and
  requires a token. The token comes from `--token`/`ETHER_SERVER_TOKEN`, or is generated
  on first start into `<data-dir>/config/server-token` (it is never logged; `--print-token`
  shows it). `--no-auth` is allowed on loopback only, and then only upgrades whose
  `Host` and (browser) `Origin` are loopback are accepted, so other web pages open on the
  machine cannot drive the engine. Use TLS (a reverse proxy) for anything beyond a
  trusted network.
- Connections must finish the upgrade and hello within 10 s (64 KiB message limit until
  then, at most 32 at once). Afterwards the server pings quiet clients and drops those
  silent for 60 s or whose writes block for 10 s; a dropped client's open gestures are
  ended and its unfinished uploads cancelled. Each client may run 4 uploads at once.
- Several UIs can connect at once: they share one project, see each other's edits live,
  and each gets only its own replies.
- Dropping audio files onto the sample browser while connected uploads them to the server
  and imports them into the project. Unfinished uploads are dropped when their client
  disconnects.
- Protocol: `crates/ether-protocol/src/remote.rs`. `ether-server --help` lists all flags.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE). Dependencies must be GPL-3-compatible (MIT, Apache-2.0, BSD, GPL).
