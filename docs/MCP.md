# AI agents and MCP (`agent-api`)

Ethereal exposes a set of **agent tools** that an LLM can call to read and edit the open
project: create tracks, write MIDI, add devices, set parameters, mix, transport, undo,
export. Two clients use them:

- the in-app AI chat (`ai-chat`), and
- `ether-mcp`, an [MCP](https://modelcontextprotocol.io) server for Claude Code, Claude
  Desktop and any other MCP client.

Both go through the same protocol commands, so they see the same tools.

## How it works

```text
Claude Code / Claude Desktop ──stdio (MCP)──► ether-mcp ──┬─ ws (remote-engine protocol) ─► desktop app (agent bridge)
                                                          ├─ ws ─► ether-server
                                                          └─ embedded headless engine (--project)
in-app AI chat ──────────────────────────────────────────────── Command::Agent ──► controller
```

- **Protocol** (`crates/ether-protocol/src/agent.rs`, CONTRACTS §13):
  `Command::Agent(ListTools)` replies `AgentTools { tools: [{ name, description, input_schema }] }`.
  `Command::Agent(CallTool { name, input })` replies
  `AgentToolResult { content, is_error }`. JSON travels as text (`input_schema`, `input`,
  `content`).
- **Registry** (`crates/ether-controller/src/agent/`): the tools live in the controller,
  so they work the same on the desktop app, `ether-server` and the web build (WASM).
  Inputs are checked against each tool's JSON Schema. An unknown tool, invalid input or
  rejected edit is a result with `is_error: true` (the model sees the message), never a
  protocol error.
- **One undo step per call.** An editing tool call runs as ordinary document commands in
  one undo step labelled `AI: <action>`. It is replicated in a collab session, and its
  patches reach every UI before the reply. The `undo` and `redo` tools use the same shared
  history as the app.
- `ether-mcp` contains no tool logic. `tools/list` and `tools/call` are forwarded to the
  engine, so a newer app brings its new tools without updating `ether-mcp`. While the app
  is unreachable, `tools/list` returns the built-in registry, and calls return an error
  that explains how to connect. The project overview is also exposed as the MCP resource
  `ethereal://project/overview`.

### Conventions

- Ids are document ids (strings) returned by the read tools (`get_project_overview` or
  `find` first) and by the creating tools. Every id argument (`track_id`, `clip_id`,
  `device_id`, id lists) also takes a **name** ("Drums", "the drum loop", "Reverb on
  Drums"): it is resolved like `find` (EN/FR aliases, a clip through its track's name, the
  user's selection, then what the agent edited last) and the result lists it under
  `resolved`; a name that matches several things equally is an error starting with
  `Ambiguous:` that lists the candidates (nothing is changed).
- Time is in **beats** (quarter notes). Arrangement positions count from the song start and
  note positions from the clip start. In 4/4 a bar is 4 beats. The tools never depend on
  bar lines: a time-signature change moves nothing.
- Volume is in dB (0 = unity, -144 = silence, max +6). Pan goes from -1 (left) to 1
  (right). MIDI pitch is 0-127 (60 = C3). Velocity is 1-127.
- Results are compact JSON. Long lists are paginated (`offset`/`limit`) or truncated, with
  a `note` that says how to get the rest.

## Tools

| Tool | What it does |
|---|---|
| `get_project_overview` | Name, tempo, time signature, beats per bar, song length, sections (markers), loop, transport (beats and bar), undo state, the user's selection, and every track with its group, mixer, devices (rack chains nested, pad counts) and clips (ids, beats, bars). |
| `find` | Fuzzy search by name across tracks, clips, devices, sections, installed plugins and built-in device types; typed matches with scores, `best` or `ambiguous`; `prefer` a kind. |
| `list_clips` | Clips filtered by track, name, kind or song range (paginated). |
| `get_track` | One track: mixer, routing, sends, devices, all clips (paginated). |
| `get_clip_notes` | The notes of a MIDI clip (paginated). |
| `list_device_types` | Built-in instruments, audio effects and MIDI effects you can add. |
| `list_plugins` | Installed plugins (the app's scanned catalog): id, name, vendor, format, category, version; `query`/`category`/`format` filters, paged, scan status. The web app has none. |
| `get_device_params` | A device's parameters (built-in or plugin): id, name, group, value, display ("1.20 kHz"), unit, range, normalized, labels; drum pads and rack chains in the header. 64 per call by default; `query` filters, `offset` pages; plugin plumbing params (MIDI CC) hidden unless `all`. |
| `get_device_param` | One parameter by name or id, with what "a bit more/less" gives. |
| `list_device_presets`, `load_device_preset` | Factory and user presets of a device (built-in or plugin); load by id or name. |
| `create_track` | `{kind: midi\|audio\|group\|return, name?}`. MIDI tracks get an instrument (default `synth`). Returns `{track_id}`. |
| `delete_track`, `rename_track` | `{track_id}`, `{track_id, name}`. |
| `set_track_mix` | `{track_id, volume_db?, pan?, mute?, solo?, arm?}`. |
| `add_device`, `remove_device` | Add a built-in `type`, an installed plugin (`plugin_id` + `format`) or a name `query` (ambiguous → candidates). Placement as in the app: an instrument replaces the track's instrument (one undo step), MIDI effects before it, audio effects last (`index` overrides). Or remove one. |
| `set_device_param` | By parameter id or name; exactly one of `value` (plain units, a label, or "2 kHz" / "-6 dB"), `normalized` (0-1), `delta`, `delta_percent` (of the knob travel). |
| `set_drum_pad`, `load_drum_pad_sample` | Edit a Drum Rack pad (volume, pan, mute, solo, choke group, name, note) by pad id, name or note; put a library sample or a clip's audio on a pad. |
| `set_rack_chain` | A rack chain's volume, pan, mute, solo, name. |
| `create_midi_clip` | `{track_id, start_beats, length_beats, name?, notes?}`. Returns `{clip_id}`. |
| `add_notes` | `{clip_id, notes: [{pitch, start_beats, duration_beats, velocity}]}`. Returns `{note_ids}`. |
| `remove_notes` | By note ids, or by clip with pitch and range filters. |
| `set_clip` | Move, resize, rename, mute or loop a clip. |
| `delete_clip`, `duplicate_clip` | Delete clips; or copy one: once, `count` times, or back to back until a span is covered (`span_seconds` through the tempo map, `span_bars`, `span_beats`, `until_beats`), one undo step. |
| `split_clip`, `transpose` | Split at song positions; transpose MIDI notes / audio clips by semitones (clips, a track, or a song range, splitting at its edges; drums skipped unless asked). |
| `set_tempo`, `set_time_signature` | `{bpm}`; `{numerator, denominator}`. |
| `transport` | `play`, `stop`, `seek` (`position_beats`), `loop` (region and on/off). |
| `undo`, `redo` | The app's shared history. |
| `save_project` | Save (headless mode: also writes the `--project` file). |
| `search_browser`, `load_browser_item` | Search the sound library and load a sample or preset. These need the browser index (`browser-v2`); without it they return an error. |
| `export_audio`, `get_export_status` | Start a background render (returns a job id) and poll it. |

The authoritative list, with full descriptions and schemas, is `tools/list` (or
`crates/ether-controller/src/agent/tools.rs`).

## Using it from Claude Code

### Where `ether-mcp` is

The desktop app ships `ether-mcp` next to its own binary:

| Install | `ether-mcp` |
|---|---|
| macOS (.dmg / .app) | `/Applications/Ethereal.app/Contents/MacOS/ether-mcp` |
| Windows (setup.exe, per user) | `%LOCALAPPDATA%\Ethereal\ether-mcp.exe` |
| Windows (.msi) | `C:\Program Files\Ethereal\ether-mcp.exe` |
| Linux (.deb / .rpm) | `/usr/bin/ether-mcp` |
| Linux (AppImage) | inside the image: run `./Ethereal_*.AppImage --appimage-extract usr/bin/ether-mcp` once and use `squashfs-root/usr/bin/ether-mcp` |

With **Allow AI agents (MCP)** on, the app's AI settings show the `claude mcp add` line with
this path filled in (copy button). The web app has no `ether-mcp`.

From a checkout, build it instead:

```sh
cargo build --release -p ether-mcp --features full   # → target/release/ether-mcp
```

Packaging: `scripts/build-sandbox-helper.mjs --bundle` (the Tauri `beforeBuildCommand`) builds it
with `--features full` and stages it in `apps/desktop/src-tauri/binaries/`. macOS and Linux
install it through `tauri.conf.json` `files` (signed on macOS by
`scripts/release/sign-macos-helper.mjs`); Windows ships it as a sidecar (`externalBin`, added by
the release overlay `scripts/release/tauri-config.mjs`, signed with the app).
`scripts/release/collect.mjs` fails the release if it is missing from the .app or the .deb.

### With the desktop app (default mode)

1. In Ethereal, turn on **Allow AI agents (MCP)** in its settings.
2. Register the server:

   ```sh
   claude mcp add ethereal -- /Applications/Ethereal.app/Contents/MacOS/ether-mcp
   ```

   (the path for your install is in the table above, or copy the line from the app's AI
   settings).

3. Ask, for example: *"make a 4-bar drum pattern on a new track"*. The edits appear in the
   app live, and each tool call can be undone with Cmd/Ctrl+Z.

If the app isn't running or the toggle is off, every tool call returns:
*"Ethereal is not reachable … turn on 'Allow AI agents (MCP)' …"*. `ether-mcp` reconnects
on the next call, so you can enable it, or restart the app, without restarting the client.

### Headless, on a project file (no app)

```sh
claude mcp add ethereal-song -- /path/to/ether-mcp --project ~/Music/song.ether
```

This mode embeds the engine with the null audio backend (`ETHER_AUDIO=null`), so nothing is
played. The `--project` argument can be:

- a project folder of an Ethereal store (`<projects root>/<uuid>/`) or the `project.ether`
  in it. It is opened in place, with its media.
- any other `.ether` path, existing or new. A new file is created named after the path.
  `save_project` writes the document there. Media imported in this mode are not kept.

### Against `ether-server`

```sh
claude mcp add ethereal-remote -- /path/to/ether-mcp --server ws://127.0.0.1:PORT --token TOKEN
```

(`ETHER_SERVER_TOKEN` also works.) A wrong token fails at start with `refused (BadToken)`.

During development, `just mcp [args]` runs `ether-mcp` from source for this checkout's dev
instance. For example, `claude mcp add ethereal-dev -- just -f $PWD/justfile mcp` attaches
to `just dev-desktop --features mcp` (or `just dev-desktop-full`): the default dev build
leaves the agent bridge out (docs/ARCHITECTURE.md "Cargo features").

## Claude Desktop

Add the server to `claude_desktop_config.json` (Settings → Developer → Edit Config):

```json
{
  "mcpServers": {
    "ethereal": {
      "command": "/Applications/Ethereal.app/Contents/MacOS/ether-mcp",
      "args": []
    }
  }
}
```

Use the path of your install (see "Where `ether-mcp` is"); in JSON a Windows path doubles its
backslashes (`"C:\\Program Files\\Ethereal\\ether-mcp.exe"`). For headless use, set
`"args": ["--project", "/path/to/song.ether"]`.

## In-app AI chat: providers

The chat (`ui/src/features/ai-chat`) works with any model provider. The tool loop
(`loop.ts`: tool calls, results, Stop, the per-turn round cap) is provider-agnostic. It talks
to a `ChatSession` (`providers/types.ts`), and each adapter keeps its own native history:

| Adapter | Providers | Transport |
|---|---|---|
| `providers/anthropic.ts` (default) | Anthropic (Claude) | `@anthropic-ai/sdk`, Messages API, prompt caching |
| `providers/openai.ts` | OpenAI, DeepSeek, Google Gemini (its OpenAI-compatible endpoint), Mistral, Groq, OpenRouter, Together, Ollama, LM Studio, any other OpenAI-compatible server | `fetch` + SSE, Chat Completions with `tools` (no SDK) |

- **The same system prompt and tools for every provider.** The registry's `input_schema`
  becomes Anthropic's `input_schema` or OpenAI's `function.parameters`. Tool results go back as
  `tool_result` blocks (with `is_error`) or as `role: "tool"` messages. Chat Completions has no
  error flag, so error results are sent as `Error: …` text.
- **Streaming tool calls.** `delta.tool_calls` fragments are assembled per `index`. A call
  sent whole without an `index` or `id` (Gemini, Mistral) works too. A call runs only once
  its arguments are complete. If they aren't valid JSON, the call isn't run and the model
  gets an error. Calls run one at a time and in order, so providers without parallel tool
  calls behave the same.
- **Models without tool support.** When a server says the model can't use tools (for
  example Ollama's "does not support tools"), the panel says so and suggests picking another
  model.
- **Settings** (per provider, in the chat's settings): base URL, model (free text with
  suggestions), API key, and "Test connection". Test connection makes a free request:
  `GET /models` (OpenAI-compatible) or `models.retrieve` (Anthropic). Keys live in
  `localStorage`. The Anthropic key keeps `eth.ai.apiKey`, and each other provider uses
  `eth.ai.apiKey.<provider>`. Keys are kept apart from the other settings, never logged, and
  sent only to that provider's base URL. Local servers (Ollama, LM Studio) need no key.
- **CORS.** Browsers only let the page call providers that allow cross-origin requests. A
  fetch that fails without a response is either an unreachable server or a CORS block, and
  the browser doesn't say which. The adapter tells them apart by repeating the request as a
  `no-cors` probe, with no key and no body. If the probe gets through, the server is up and
  the panel says the provider blocked the request. It adds a fix for that provider: set
  `OLLAMA_ORIGINS` for Ollama, turn on "Enable CORS" in LM Studio, or use OpenRouter or a
  proxy for cloud APIs. This is the web build only: the desktop app doesn't apply CORS (next
  point).
- **Desktop: requests from Rust.** On the desktop, both adapters fetch through
  `tauri-plugin-http` (`providers/fetch.ts`): the request is made in Rust, so providers that
  refuse web origins work, and responses still stream chunk by chunk. No `Origin` header is
  sent (Ollama refuses origins it doesn't know). The plugin only fetches URLs in its scope:
  the preset providers' default base URLs (`capabilities/ai-providers.json`), plus the origin
  of a base URL the user configured, which the UI registers first with the
  `ai_http_allow(base_url)` command (`src-tauri/src/ai_http.rs`). That command accepts http(s)
  URLs with a plain host name or IPv4 address and no user name or password, and allows that
  origin only (`<origin>/*`) until the app quits. Redirects are checked against the scope
  too (`scopeRedirects`). Every other URL is refused. Keys are never logged; the plugin
  has no cookie jar.

## Desktop agent bridge

The bridge is off by default. The setting is stored in `<app data dir>/config/agent.json`.
Tauri commands (the toggle UI belongs to `ai-chat`):

- `agent_bridge_status() -> { enabled, port, connected_clients }`
- `agent_bridge_set_enabled(enabled: bool) -> { enabled, port, connected_clients }`

When the bridge is enabled, the app listens on **127.0.0.1** on a random port. It speaks the
remote-engine protocol (CONTRACTS §11.5) through `ether-server`'s own listener and router,
used as a library (`ether_server::agent_bridge`), against the same engine and controller as
the UI. The app then writes the runtime file:

```json
{ "port": 51234, "token": "<48 hex chars>", "pid": 4242, "version": "0.0.1" }
```

The runtime file is `<app data dir>/agent-bridge.json`:

| Build | `<app data dir>` |
|---|---|
| release | `<data>/ws.ethereal.daw/` |
| dev (`just dev-desktop`) | `<data>/ethereal-dev/<instance>/` |

Here `<data>` is `~/Library/Application Support` on macOS, `$XDG_DATA_HOME` or
`~/.local/share` on Linux, and `%APPDATA%` on Windows. `ether-mcp` checks the dev file of
`$ETHER_INSTANCE` (default `default`) first, then the release one. `--runtime-file`
overrides both.

Disabling the bridge, or quitting the app, closes the listener (which drops connected
agents) and deletes the file.

## Security model

- **Off by default, explicit opt-in.** Nothing listens until the user enables the toggle.
- **Loopback only.** The bridge binds `127.0.0.1` and is never reachable from the network.
- **Token.** Each enable creates a fresh 24-byte random token. Clients must send it in the
  hello (compared in constant time, never logged). A wrong or missing token is rejected
  (`BadToken`, close code 4001).
- **Owner-only discovery.** The runtime file is created with mode `0600` (Unix), so other
  local users can't read the token. On Windows it inherits the per-user app data folder's
  ACL. It is deleted on disable and on quit; a stale file left by a crash is removed at the
  next start.
- **Same hardening as `ether-server`.** The bridge has the same handshake deadline, message
  limits, idle timeout and per-client upload cap (README "Remote engine").
- **What an agent can do.** An agent can do what the tools allow: edit the open project
  (each edit can be undone), control the transport, save, and export audio into the
  project folder. It can't read or write arbitrary files, run commands, or open other
  projects. The bridge speaks the full remote-engine protocol, though, so a client holding
  the token can send any command the UI can. Treat the token like the app itself.
- **Prompt injection.** Tool results contain user-controlled text (track and clip names). The
  MCP client shows the model's tool calls, and every edit can be undone.

## Follow-ups

- Package `ether-mcp` in the release bundle (next to the app binary) and show the
  `claude mcp add` command in the app (out of scope for `agent-api`).
- `search_browser` and `load_browser_item` light up once the browser index is available on
  the host.
- AI chat: carry a conversation over when the provider changes. Today the new provider
  starts fresh, and the panel says so.
