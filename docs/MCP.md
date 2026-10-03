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

- Ids are document ids (strings) returned by the read tools (`get_project_overview` first)
  and by the creating tools.
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
| `get_project_overview` | Name, tempo, time signature, loop, transport, undo state, the user's selection, and every track with its mixer, devices and clips (ids, beats). |
| `get_track` | One track: mixer, routing, sends, devices, all clips (paginated). |
| `get_clip_notes` | The notes of a MIDI clip (paginated). |
| `list_device_types` | Built-in instruments, audio effects and MIDI effects you can add. |
| `get_device_params` | A device's parameters: id, name, unit, range, value, labels. 64 per call by default (`total` + a `note` when more exist); `query` filters by name/group, `offset` pages. |
| `create_track` | `{kind: midi\|audio\|group\|return, name?}`. MIDI tracks get an instrument (default `synth`). Returns `{track_id}`. |
| `delete_track`, `rename_track` | `{track_id}`, `{track_id, name}`. |
| `set_track_mix` | `{track_id, volume_db?, pan?, mute?, solo?, arm?}`. |
| `add_device`, `remove_device` | Add a built-in device to a chain (at `index`), or remove one. |
| `set_device_param` | By parameter id or name; the value is in plain units, or a label for choices. |
| `create_midi_clip` | `{track_id, start_beats, length_beats, name?, notes?}`. Returns `{clip_id}`. |
| `add_notes` | `{clip_id, notes: [{pitch, start_beats, duration_beats, velocity}]}`. Returns `{note_ids}`. |
| `remove_notes` | By note ids, or by clip with pitch and range filters. |
| `set_clip` | Move, resize, rename, mute or loop a clip. |
| `delete_clip`, `duplicate_clip` | Delete clips, or copy one (right after it by default). |
| `set_tempo`, `set_time_signature` | `{bpm}`; `{numerator, denominator}`. |
| `transport` | `play`, `stop`, `seek` (`position_beats`), `loop` (region and on/off). |
| `undo`, `redo` | The app's shared history. |
| `save_project` | Save (headless mode: also writes the `--project` file). |
| `search_browser`, `load_browser_item` | Search the sound library and load a sample or preset. These need the browser index (`browser-v2`); without it they return an error. |
| `export_audio`, `get_export_status` | Start a background render (returns a job id) and poll it. |

The authoritative list, with full descriptions and schemas, is `tools/list` (or
`crates/ether-controller/src/agent/tools.rs`).

## Using it from Claude Code

Build the server once:

```sh
cargo build --release -p ether-mcp      # → target/release/ether-mcp
```

### With the desktop app (default mode)

1. In Ethereal, turn on **Allow AI agents (MCP)** in its settings.
2. Register the server:

   ```sh
   claude mcp add ethereal -- /path/to/ethereal/target/release/ether-mcp
   ```

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
to `just dev-desktop`.

## Claude Desktop

Add the server to `claude_desktop_config.json` (Settings → Developer → Edit Config):

```json
{
  "mcpServers": {
    "ethereal": {
      "command": "/path/to/ethereal/target/release/ether-mcp",
      "args": []
    }
  }
}
```

For headless use, set `"args": ["--project", "/path/to/song.ether"]`.

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
| release | `<data>/dev.ethereal.app/` |
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
