---
title: ai agents (mcp)
order: 20
summary: Let Claude Code, Claude Desktop or another MCP client edit your project in the desktop app, or a project file with no app at all.
platform: desktop
audience: advanced
covers:
  - ai-mcp/external-ai-agents-mcp #allow-external-ai-agents
  - ai-mcp/mcp-headless-and-remote-modes #without-the-app-project-files-and-engine-servers
---

Besides the built-in [ai assistant](/docs/advanced/ai-assistant), AI apps on your computer can drive Ethereal through MCP (the Model Context Protocol). Claude Code, Claude Desktop and other MCP clients can then create tracks, write MIDI, add devices, mix, play and export, live in your open project. This works with the desktop app. The web app can't do it.

## allow external ai agents

It is off by default. To turn it on:

1. Open the **ask ai** tab in the left rail and click **ai settings** (the gear).
2. Turn on **allow external ai agents (mcp)**.

The section then shows the local port it listens on, how many agents are connected, and a `claude mcp add` command with a copy button.

For safety, it only accepts connections from this computer (never from the network), and each time you turn it on it makes a new secret token that only your user account can read. Turning it off, or quitting Ethereal, disconnects every agent.

> [!warning]
> An agent can do anything you can do in the open project: edit it, play, save and export into the project folder. It can't open other projects or touch other files. Only connect agents you trust.

## connecting claude code

The desktop app installs the MCP server, `ether-mcp`, next to the app. With the setting on, copy the command from the AI settings and run it in a terminal. It looks like:

```
claude mcp add ethereal -- /Applications/Ethereal.app/Contents/MacOS/ether-mcp
```

Where `ether-mcp` is:

| install | `ether-mcp` |
|---|---|
| macOS | `/Applications/Ethereal.app/Contents/MacOS/ether-mcp` |
| Windows (installer, per user) | `%LOCALAPPDATA%\Ethereal\ether-mcp.exe` |
| Windows (.msi) | `C:\Program Files\Ethereal\ether-mcp.exe` |
| Linux (.deb, .rpm) | `/usr/bin/ether-mcp` |

With the Linux AppImage, the program is inside the image, so the app shows the command without a path. Extract it once with `./Ethereal_*.AppImage --appimage-extract usr/bin/ether-mcp` and use the extracted file.

For Claude Desktop, add the same path as an MCP server in its configuration file (Settings > Developer > Edit Config).

Then ask the agent for an edit, for example "make a 4-bar drum pattern on a new track". Each tool call is one undo step, which you can undo in Ethereal with [[action:edit.undo]].

If Ethereal is closed or the setting is off, the agent's calls fail with a message saying Ethereal is not reachable. Turn the setting on (or start the app) and try again: you don't need to restart the agent.

## without the app: project files and engine servers

`ether-mcp` has two more modes, for scripts and automation:

- `ether-mcp --project song.ether` opens a project file directly, with no app and no sound. A path that doesn't exist yet creates a new project. Saving writes the file.
- `ether-mcp --server ws://host:port --token TOKEN` drives an [engine server](/docs/advanced/engine-server).

## where next

- [the ai assistant](/docs/advanced/ai-assistant): the chat built into Ethereal.
- [the engine server](/docs/advanced/engine-server): run the engine on another machine.
- [undo and history](/docs/basics/undo-and-history)
