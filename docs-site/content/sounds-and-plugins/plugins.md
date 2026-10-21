---
title: plugins
order: 20
summary: Use your CLAP, VST3 and Audio Unit plugins in the desktop app, and keep projects safe when a plugin crashes or is missing.
platform: desktop
audience: everyone
covers:
  - plugins/plugins-are-desktop-only #plugins-need-the-desktop-app
  - plugins/plugin-folders-and-scanning #where-ethereal-looks-for-plugins
  - plugins/failed-plugins-and-retry #plugins-that-failed-to-load
  - plugins/same-plugin-in-several-formats #the-same-plugin-in-several-formats
  - plugins/plugin-editor-windows #opening-a-plugins-window
  - plugins/linux-plugin-editors-need-x11 #plugin-windows-on-linux
  - plugins/keys-in-plugin-windows #keys-in-plugin-windows
  - plugins/sandboxing-and-crashes #when-a-plugin-crashes
  - plugins/open-without-plugins-safe-mode #opening-a-project-without-plugins
  - plugins/missing-plugins #missing-plugins
  - plugins/plugin-state-is-saved-in-the-project #how-plugin-sounds-are-saved
  - plugins/plugin-presets #plugin-presets
---

Ethereal's desktop app runs the instruments and effects you already own as plugins. This page covers where Ethereal finds them, how to open their windows, and what happens when a plugin crashes or isn't installed. To add a plugin to a track, use the same **+** buttons as for built-in devices (see [devices and presets](/docs/making-music/devices-and-presets)).

## plugins need the desktop app

The desktop app hosts plugins in these formats:

- **CLAP**
- **VST3**
- **AU** (Audio Units, on macOS only)

The web version can't load plugins. Its device library lists only Ethereal's built-in devices, and its **settings › plugins** tab just says that plugins are loaded by the desktop app. If you open a project that uses plugins in the browser, those devices are bypassed and the tracks play without them.

## where ethereal looks for plugins

Open **settings**, then **plugins › folders & scan**.

- **system folders**: the standard plugin folders of your computer, one row per format. A folder that doesn't exist is marked **not found**. The **scan** switch is on by default. Turning it off skips these folders and, on macOS, Audio Units.
- **your folders**: click **add folder…** to scan a folder of your own, for example a drive where you keep plugins. Each folder can be limited to one format, or left on **any format**. Click **×** to remove a folder.

Adding, removing or changing a folder starts a scan. You can also scan by hand:

- **rescan** loads only plugins that are new or have changed since the last scan. It's fast.
- **full rescan** loads every plugin again, including the ones that failed before.

Each plugin is scanned in its own process, several at a time. A plugin that crashes or hangs while it's scanned can't take the app down: it's just listed as failed.

### plugins that failed to load

When some plugins couldn't be loaded, the scan row says how many and shows **failed plugins (n)…**. The list there gives each plugin's path and why it failed. Right-click a plugin and choose **retry…** to scan it again with a longer timeout: 90 seconds by default, up to an hour. Big sample libraries and shell plugins that bundle hundreds of plugins can take several minutes.

## the same plugin in several formats

If a plugin is installed in several formats (for example as CLAP, VST3 and AU), the device library lists it once, in your preferred format: CLAP first, then VST3, AU and VST2. You choose it in **settings › plugins › plugin list**, under **preferred format**. To pick a specific format of a plugin, turn off **show each plugin once**: the library then lists each format, each with a format chip. The import resolver and **replace device…** always list every format.

When the list holds two or more formats, toggles for each format appear at the top of the library. Turn off the formats you don't use to hide them, in the list and in its search. At least one format always stays on, and Ethereal remembers your choice.

To hide a plugin, right-click it in the device library and choose **hide …** (every format) or **hide this … version** (that format only: another format of the plugin is listed instead). The toast that follows can undo it. The hidden plugins are listed under **hidden plugins** in **settings › plugins › plugin list**, where **×** shows one again.

## opening a plugin's window

Click the window button in a plugin's header to open the plugin's own interface. It opens in a separate window that stays above Ethereal's main window. Click the button again to close it. The plugin's card in Ethereal also lists its parameters, with the plugin's own value text (like "250 ms" or "-6 dB").

On Windows and Linux, a CLAP plugin's window opens only if the plugin can open a window of its own. Many CLAP plugins can only draw into a window the host makes, which Ethereal does only on macOS for now: their window doesn't open, but their parameters stay on their card. VST3 windows open on every system.

### plugin windows on linux

On Linux, plugin windows need an X display. On a Wayland desktop that means XWayland.

### keys in plugin windows

While a plugin window has the focus:

- [[action:transport.play]] still plays and stops Ethereal, and the other play shortcuts still work.
- When the [computer MIDI keyboard](/docs/making-music/computer-midi-keyboard) is on, its note keys play Ethereal's track instead of going to the plugin.
- Key combinations with [[key:Ctrl]], [[key:Alt]] or [[key:Mod]] always go to the plugin, so its own shortcuts keep working.

## when a plugin crashes

By default, plugins run inside Ethereal. For a plugin you don't trust, right-click its header and choose **run sandboxed (crash-safe, +1 block latency)**. The plugin then runs in its own process. If it crashes, Ethereal keeps playing: the device shows **crashed · bypassed** and the track plays without it. Click **⟳** to reload the plugin from its last saved state.

Sandboxing adds one audio block of latency. To switch back, right-click the header and choose **run in-process (no sandbox)**.

> [!note]
> Sandboxing works on macOS and Linux. On Windows, plugins always run inside Ethereal: choosing to run a plugin sandboxed does nothing there.

## opening a project without plugins

If a project crashes or hangs because of a plugin, open it without plugins. Right-click the project in the project list (or click its **…** button) and choose **open without plugins**. The crash recovery dialog offers the same button. Every plugin is kept as a bypassed placeholder with its settings saved, marked **safe mode · bypassed**, and the status bar at the bottom says **plugins disabled (safe mode)**. Then:

- click **⟳** on one device to load just that plugin, or
- click **load plugins** in the status bar to load them all.

See [safe mode](/docs/advanced/safe-mode) for more.

## missing plugins

If a project uses a plugin that isn't installed on this computer, Ethereal keeps the device and its saved sound. It's marked **missing · bypassed** and the track plays without it. Nothing is lost:

1. Install the plugin, then **rescan** in **settings › plugins**.
2. The device now shows **found · reload**. Click it to load the plugin with the sound saved in the project.

To use another plugin instead, right-click the **missing · bypassed** badge or the device's header and choose **resolve…**. Right-click any device's header and choose **replace device…** to swap it for another device of the same kind (an effect for an effect, an instrument for an instrument), factory or plugin: a popover under the header lists them like the device library, and a click replaces at once. With **retain automations and parameters** (hover it for what it keeps), values and automation carry over to parameters with the same name; racks can't be replaced. See [missing plugins and samples](/docs/sounds-and-plugins/missing-plugins-and-samples).

## how plugin sounds are saved

Each plugin's full state is saved in the project file. Ethereal records which plugin it is (its format and id), not where it's installed, so the project opens on any computer that has the plugin. When you quit, Ethereal saves the plugins' current state with the project.

## plugin presets

The preset menu in a device's header works for plugins too: it saves and loads the plugin's complete state. Plugin presets are stored per plugin in your user library, next to your other presets. Loading one is a single undo step. See [devices and presets](/docs/making-music/devices-and-presets).

## where next

- [Devices and presets](/docs/making-music/devices-and-presets): add and arrange devices on a track.
- [Safe mode](/docs/advanced/safe-mode): open a project that crashes.
- [Missing plugins and samples](/docs/sounds-and-plugins/missing-plugins-and-samples): replace plugins a project uses that aren't installed.
