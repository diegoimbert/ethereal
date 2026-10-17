<p align="center"><img src="assets/brand/ethereal-logo.svg" alt="" width="96"></p>

# Ethereal

Ethereal is a DAW built around a single arrangement view (there is no
session/clip-launcher view). The audio engine is written in Rust and the UI in React/TypeScript. The same UI runs in a desktop app (macOS, Windows and
Linux, via Tauri) and in the browser at [ethereal.ws](https://ethereal.ws), where the engine
runs as WebAssembly.

## Status

Ethereal is in active development. Installers for macOS (arm64 and x64), Windows and Linux,
plus the web build, are on the [Releases page](https://github.com/diegoimbert/ethereal/releases).
Report bugs and request features in the [issue tracker](https://github.com/diegoimbert/ethereal/issues).

What works today:

- **Arrangement editing**: audio and MIDI tracks, piano roll, automation, warping and time
  stretching, recording and comping, groups, sidechain, built-in instruments and effects,
  racks and a drum rack.
- **Plugin hosting** (desktop): CLAP, VST3 and AU (macOS), scanned out of process and
  optionally sandboxed. See [docs/PLUGIN-FORMATS.md](docs/PLUGIN-FORMATS.md)
- **Audio export** to WAV, FLAC and MP3, a headless remote engine (`ether-server`) and an
  MCP server that lets AI agents edit projects (both below).

Developer docs: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) (architecture, Cargo
features), [docs/CONTRACTS.md](docs/CONTRACTS.md) (protocol, model, engine traits),
[docs/RELEASING.md](docs/RELEASING.md). Third-party licenses: the components shipped in the
releases and their license texts are in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md),
regenerated with `just notices` (the license policy gate is `deny.toml`).

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
| `just dev-desktop` | Tauri desktop app, fast dev core (see "Cargo features" below). `just dev-desktop-headless` = same with the null audio backend. Extra args go to `tauri dev` (`just dev-desktop --features plugins`). |
| `just dev-desktop-full` | Desktop app with every feature (what releases ship), plugin helpers included. |
| `just dev-server` / `just dev-server-full` | Headless engine over WebSocket (`ether-server`) for the web UI's remote engine setting (see below), dev core / every feature. |
| `just e2e-web [args]` / `just e2e-native` | Playwright end-to-end tests of the web build (run `just e2e-install` once) / native smoke test on the null audio backend. |
| `just mcp [args]` | MCP server (`ether-mcp`) for Claude Code / Claude Desktop (see below and [docs/MCP.md](docs/MCP.md)). |
| `just gen-types` | Regenerate `ui/src/generated/` (TS types) from `crates/ether-protocol`. Commit the result; CI fails if stale. |
| `just notices` | Regenerate `THIRD-PARTY-NOTICES.md` (the third-party licenses shipped in the releases; `third-party/` holds texts the crates omit). Commit the result; CI fails if stale. |
| `just check-licenses` | `cargo deny check licenses` — every dependency's license must be on the `deny.toml` allow-list. |
| `just check-wasm` | `cargo check --target wasm32-unknown-unknown` for the engine-side crates (every feature, and none). |
| `just check-features` | clippy over the workspace with `--all-features` and with `--no-default-features` (part of `check-all`). |
| `just check-all` / `just test-all` | Everything the gates run, with every Cargo feature. |
| `just dev-port` | Prints this checkout's instance id and ports. |

## Cargo features (fast dev builds)

By default the Rust crates build a **dev core**. It leaves out the plugin format hosts and
sandbox, and Signalsmith complex warp. Every feature that is off replies `Unsupported`, or
the UI reports it as "not available in this build". Releases and the gates build `full`.
The feature map is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#cargo-features).

```sh
just dev-desktop                       # dev core
just dev-desktop --features plugins    # dev core + plugin hosting (also: warp-complex, mcp)
just dev-desktop-full                  # everything, like a release
cargo test -p ether-native             # dev core; gated tests are skipped (required-features)
cargo test -p ether-native --features full
just check-all && just test-all        # every feature (what the gates run)
```

When you change gated code, build it with its feature (or `full`) before you push. `just
check-all` checks both `--all-features` and `--no-default-features`.

## Layout

```
crates/
  ether-protocol        commands/events/replies, TS type export      (native + wasm)
  ether-model           document types, IDs, ops, undo, .ether format (native + wasm)
  ether-core            RT engine: graph, scheduler, transport, mixer (native + wasm)
  ether-devices         built-in instruments and effects             (native + wasm)
  ether-media           decode, resample, peaks                      (native + wasm)
  ether-controller      commands -> ops -> patches; model -> engine  (native + wasm)
  ether-stretch         Stretcher trait + Signalsmith                (trait wasm, impl native)
  ether-plugin-host     plugin format trait, registry, scanning      (native)
  ether-clap, ether-vst3, ether-au              format hosts         (native)
  ether-plugin-scanner  out-of-process scanner binary                (native)
  ether-sandbox         out-of-process plugin sandbox                (native)
  ether-native          cpal/null audio host, threads                (native)
  ether-server          headless engine over WebSocket               (native)
  ether-mcp             MCP server for AI agents                     (native)
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
scripts/                dev-env.mjs (instance/ports), feat.mjs (the ee/ workflow), release tooling
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
  web UI (`just dev-web`), open Settings › Advanced › **Engine server** and enter
  `ws://127.0.0.1:<port>` and the token. Disconnecting returns to the in-browser engine.
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

## AI agents (MCP)

`ether-mcp` lets Claude Code, Claude Desktop or any other MCP client edit a project: create
tracks, write MIDI, add devices, mix, play and export. Every tool call is one undo step. It
uses the same agent tools as the in-app AI chat.

- **With the desktop app:** turn on **Allow AI agents (MCP)** in the app, then
  `claude mcp add ethereal -- <path>/ether-mcp`. Releases ship `ether-mcp` next to the app
  binary (paths per platform in [docs/MCP.md](docs/MCP.md); the app's AI settings show the
  line); from a checkout, `cargo build --release -p ether-mcp --features full`. The bridge listens on loopback only, with a
  random token in an owner-only runtime file.
- **Headless:** `ether-mcp --project song.ether` (no app, no audio output;
  `save_project` writes the file). **Remote:** `ether-mcp --server ws://HOST:PORT --token T`.
- `just mcp` runs it from source for this checkout's dev instance (attach it to
  `just dev-desktop --features mcp` or `just dev-desktop-full`).

Setup for Claude Desktop, the tool list and the security model: [docs/MCP.md](docs/MCP.md).

## License

Ethereal is licensed under the **GNU Affero General Public License, version 3 or later**
(AGPL-3.0-or-later) — see [LICENSE](LICENSE). A separate commercial or alternative license is
available for those who need it (contact [@diegoimbert](https://github.com/diegoimbert)); see
[LICENSING.md](LICENSING.md) for the dual-licensing model.

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to set up a checkout, what CI expects and how
to get a change merged. Everyone taking part agrees to the
[Code of Conduct](CODE_OF_CONDUCT.md).

Contributions require signing the [Contributor License Agreement](CLA.md), which is enforced
automatically on pull requests (the `CLA` check must pass before a pull request can merge).

Ethereal is built with third-party open-source software; the components and their license
texts are in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
