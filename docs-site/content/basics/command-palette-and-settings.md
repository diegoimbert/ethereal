---
title: the command palette and settings
order: 80
summary: Run any command by typing its name, find the commands that live only in the palette, and get around the settings.
platform: all
audience: beginner
covers:
  - app-shell/command-palette-cmd-ctrl-k #the-command-palette
  - shortcuts-commands/command-palette-k-ctrl-k #the-command-palette
  - shortcuts-commands/commands-only-in-the-palette #commands-only-in-the-palette
  - app-shell/settings-overview #settings
---

The command palette is a search box for everything the app can do. It's the quickest way to find a command when you don't know where its button is, and a few commands are only there. This page also gives a tour of the settings.

## the command palette

Press [[action:palette.open]], or click the last button of the left rail, to open the palette. Then:

1. Type part of a command's name. The letters don't have to be next to each other: "mtr" finds **turn metronome on**. When the app is in another language, a command's English name finds it too.
2. Use [[key:ArrowUp]] and [[key:ArrowDown]] to move through the results.
3. Press [[key:Enter]] to run the highlighted command, or [[key:Escape]] to close the palette.

Commands you ran recently come first. When a command has a keyboard shortcut, the palette shows it, following your own [keyboard shortcuts](/docs/reference/keyboard-shortcuts).

The palette doesn't open while you're typing in a text field, so the shortcut never gets in the way of your typing.

The palette lists commands for tracks, the transport, editing, the project, panels, templates, devices (**add …** inserts a device on the selected track), sharing, the appearance and help.

## commands only in the palette

These commands have no button or menu entry anywhere else:

- **print keyboard shortcuts**;
- **mouse and wheel settings…**;
- **save as…**, **duplicate project**, **rename project** and **new project from template…**;
- **relink missing samples…** and **collect all and save**;
- **zoom to fit**, to see the whole song;
- **go to track …**, to jump to a track by name;
- **pin** and **unpin** the browser, the inspector and the editor drawer;
- **name checkpoint…** (see [undo and history](/docs/basics/undo-and-history#checkpoints));
- **turn MIDI mode on** and **turn MIDI mode off** (see [MIDI learn](/docs/advanced/midi-learn)) [[desktop]];
- **switch to light theme** and **switch to dark theme**;
- **debug: …**, the debug panels (for troubleshooting).

## settings

The **gear** at the right end of the top bar opens the settings. They're split into tabs:

| tab | what's in it |
|---|---|
| **general** | **startup** (reopen the last project on launch, see [projects and saving](/docs/basics/projects#reopening-your-last-project)); in the browser, **storage** (whether the browser keeps your projects safe, see [where your projects live](/docs/welcome/where-your-projects-live)); **library files** (which files the browser lists); **updates** and **about**. |
| **appearance** | the theme (light or dark), the interface zoom, animations, the colour blobs and glow, and translucency. |
| **audio** | the driver, output and input devices, sample rate and buffer size, and an input check. In the browser, the system picks the devices. See [audio and MIDI setup](/docs/making-music/audio-and-midi-setup). |
| **input** | the mouse wheel and buttons: zoom and scroll sensitivity, which modifier zooms, inverted directions, the middle button and the back and forward buttons. See [zoom and scroll](/docs/basics/playing-and-navigating#zoom-and-scroll). **edit keyboard shortcuts…** opens the shortcuts editor (also **keyboard shortcuts…** in the palette). |
| **plugins** | [[desktop]] the folders scanned for plugins, and rescanning. See [plugins](/docs/sounds-and-plugins/plugins). |
| **sharing** | your sharing preferences, such as resuming sharing when you open a shared project. See [live sessions](/docs/sharing/live-sessions). |
| **privacy** | crash reports and usage statistics, what's sent, and news. See [privacy and telemetry](/docs/reference/privacy-and-telemetry). |
| **advanced** | the sharing servers, the static share storage, the engine server (once turned on), the relay session, the engine log (desktop) and the debug panels. See [the settings reference](/docs/advanced/settings). |

Appearance, input and privacy settings are kept on this computer (or in this browser). The other settings need the audio engine to be running.

## where next

- [Keyboard shortcuts](/docs/reference/keyboard-shortcuts)
- [The settings reference](/docs/advanced/settings)
- [A tour of the screen](/docs/basics/the-screen)
