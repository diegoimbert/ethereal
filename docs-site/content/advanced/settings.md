---
title: settings reference
order: 90
summary: Every page of the settings window and what its options do, with links to the pages that explain them.
platform: all
audience: everyone
covers:
  - app-shell/debug-panels-advanced #advanced
---

Open the settings with the gear button at the right end of the status bar (at the bottom), or run **audio settings…** or **mouse and wheel settings…** from the [command palette](/docs/basics/command-palette-and-settings). The settings are in categories (the list on the left); a category with several pages lists them under it when you open it. Every change applies at once, and **done** closes the window. Options that need the audio engine say **no engine connected** when it isn't running.

Some settings belong to a project (tempo, swing, MIDI mappings) and live in the project, not here.

## general

**language & startup**:

- **language**: the interface's language, on this device.
- **startup**: **reopen last project on launch** (off by default). See [projects and saving](/docs/basics/projects).

**library**:

- **storage** [[web]]: whether the browser has agreed to keep your projects, how much space they use, and **request again** if it hasn't. See [where your projects live](/docs/welcome/where-your-projects-live).
- **library files**: which files the browser lists. Each rule is a kind and its text: a **file type** (extensions such as `wav, aif`), or a name that **starts with**, **ends with**, **is** or **contains** some text. **use regex (advanced)** shows the rules as regular expressions instead. **reset to defaults** asks first, then restores the defaults. See [the browser](/docs/sounds-and-plugins/browser).

## appearance

- **theme**: dark or light.
- **zoom**: scales the whole interface (text, controls and panels).
- **animations**: classic, fast (every animation takes half the time) or reduced motion (changes apply at once).
- **color blobs**: the soft colors drifting behind dialogs and, faintly, behind the arrangement.
- **glow**: the halo around selected notes, clips and tracks, the playhead and status lights.
- **translucency**: how see-through panels are, or **disable translucency**.

These are kept on this device.

## audio

The audio driver, the **output device**, the **input device**, the **sample rate** and the **buffer size**, the engine's status and an input check. In the browser, your system chooses the devices. See [audio and midi setup](/docs/making-music/audio-and-midi-setup).

## mouse & keyboard

**mouse & trackpad**: how the mouse wheel and buttons behave, kept on this device:

- **wheel**: zoom and scroll sensitivity, whether the plain wheel scrolls or zooms, which modifier does the other, inverted zoom and scrolling, and whether [[key:Shift]] + wheel scrolls sideways.
- **mouse buttons**: what a middle-button drag does (pan the view, or nothing) and what the back / forward buttons do (nothing, or undo / redo).

**keyboard shortcuts**: **edit keyboard shortcuts…** opens the shortcuts editor. See [keyboard shortcuts](/docs/reference/keyboard-shortcuts).

## plugins [[desktop]]

The folders scanned for plugins: the system folders (each can be turned off) and your own folders, plus the scan buttons. In the web version this page only says plugins need the desktop app. See [plugins](/docs/sounds-and-plugins/plugins).

## advanced

For special setups. Most people never change these. **engine**:

- **engine server**: run this window on an engine on another machine. Hidden until you run **engine server…** from the command palette (or used one before). See [the engine server](/docs/advanced/engine-server).

**diagnostics**:

- **engine log** [[desktop]]: **open engine log** shows what the engine reports (warnings, plugin problems, why it stopped responding) in a terminal window. See [troubleshooting](/docs/reference/troubleshooting).
- **debug panels**: tools for diagnosing problems, mostly for developers.

## other settings

A few settings live where you use them:

- the AI provider, model, key and the external agents switch: in the **ask ai** panel's settings. See [the ai assistant](/docs/advanced/ai-assistant).
- keyboard shortcuts: **edit keyboard shortcuts…** under **mouse & keyboard**, or **keyboard shortcuts…** in the command palette. See [keyboard shortcuts](/docs/reference/keyboard-shortcuts).
- the metronome: right-click the metronome button in the transport bar. See [playing and moving around](/docs/basics/playing-and-navigating).

## where next

- [audio and midi setup](/docs/making-music/audio-and-midi-setup)
- [troubleshooting](/docs/reference/troubleshooting)
