---
title: troubleshooting
order: 30
summary: What to check when there's no sound, the audio crackles, the engine stalls, a plugin misbehaves or your web projects are at risk.
platform: all
audience: everyone
covers:
  - app-shell/engine-not-responding-warning #the-engine-is-not-responding
  - app-shell/engine-log-desktop #the-engine-log
---

Find your problem in the headings below. Each part says what's usually going on and what to try. If nothing here helps, [report an issue](https://github.com/diegoimbert/ethereal/issues) and say what your setup is.

## no sound

Work through these in order:

1. **Is it playing?** Press [[action:transport.play]] and watch the playhead move. If it doesn't move at all, see [the engine is not responding](#the-engine-is-not-responding).
2. **Do the meters move?** Track headers show a level meter. No level on a track means nothing reaches it: check that the clip isn't muted, that the track isn't muted, and that no other track is soloed.
3. **Is the output right?** [[desktop]] Open **settings** (the gear in the top bar), then **audio**, and check **output device**. Use the **refresh devices** button after plugging in an interface or headphones.
4. **In the browser** [[web]], the system picks the output device: change it in your operating system or browser, not in Ethereal. Browsers only allow sound after you interact with the page, so click or press a key once if the first playback is silent.

### the output device disappeared [[desktop]]

If you unplug your audio interface or switch off Bluetooth headphones while Ethereal runs, you get a warning that names the device. Ethereal keeps going on your configured output if it can, else on the system's default output, and else without sound (the app keeps working). It looks for your device again and switches back when it's plugged in. Your saved settings don't change.

### "device closed" [[desktop]]

If the audio tab says **device closed: no local engine plays**, this window is playing through an [engine server](/docs/advanced/engine-server). Your audio device opens again, with the same settings, when you switch back to your own engine.

## crackles and dropouts

A crackle or short gap is a **dropout**: the computer didn't finish the next piece of audio in time. The status line in **settings › audio** counts them (**… dropouts**) next to the sample rate, the buffer size and the latency.

- **Raise the buffer size** in **settings › audio**. A bigger buffer gives the computer more time per piece, at the cost of more delay between playing a key and hearing it. Lower it again for recording, raise it for mixing.
- **Freeze heavy tracks**: freezing renders a track so its devices stop using the cpu. See [freeze, flatten and bounce](/docs/advanced/freeze-and-bounce).
- **Close other audio apps** and anything heavy running in the background.
- On a laptop, plug in the power: battery saving slows the cpu down.

The buffer size you choose is a request: some devices run another size. The audio tab shows what the device actually uses. See [audio and MIDI setup](/docs/making-music/audio-and-midi-setup).

## the engine is not responding

When the engine (the part that makes the sound) stalls, a warning says **the engine is not responding**, with the reason when it knows it, for example a plugin that's busy opening its editor. Audio keeps playing, and what you do in the meantime runs as soon as it recovers. When it's over, a short message says how long it lasted.

A stall of a second or two while a big plugin opens is normal. If it happens often, or never ends:

- Check the [engine log](#the-engine-log) [[desktop]]: it says why the engine stalled.
- If a plugin is the cause, try running it sandboxed (macOS and Linux), or open the project in [safe mode](/docs/advanced/safe-mode) (see [plugins crash or fail to load](#plugins-crash-or-fail-to-load)).
- [Report an issue](https://github.com/diegoimbert/ethereal/issues) with what the engine log says.

### the audio engine didn't start

If Ethereal can't start its engine at all, it shows **the audio engine didn't start** with the reason. Your projects are safe: nothing was opened. Use **retry** or **reload**, and [report an issue](https://github.com/diegoimbert/ethereal/issues) if it keeps happening. [[web]] If the page says **a new version of ethereal is out**, reload the page to get it.

## the engine log [[desktop]]

**Settings › advanced › engine log › open engine log** opens a terminal window that follows the engine's log as it runs. It shows warnings, plugin problems and why the engine stopped responding, if it did. Copy the end of it into your report when you report a problem.

## plugins crash or fail to load [[desktop]]

Plugins are desktop only. The web version has built-in devices only.

- **A plugin doesn't show up**: open **settings › plugins**, check that its folder is scanned and that the right formats are on, then **rescan**. **failed plugins (…)…** lists the plugins that couldn't load, with the reason; right-click one to retry it with a longer timeout. See [plugins](/docs/sounds-and-plugins/plugins).
- **A plugin crashes**: on macOS and Linux, right-click its header and choose **run sandboxed (crash-safe, +1 block latency)**. It then runs in its own process: if it crashes, the device shows that it crashed and is bypassed, with a reload button, and the rest of Ethereal keeps running.
- **A project crashes as soon as it opens**: open it without plugins. Right-click the project in the recent projects list (or use the crash recovery dialog) and choose **open without plugins**. Then load the plugins one by one to find the culprit. See [safe mode](/docs/advanced/safe-mode).
- **A plugin is missing** on this computer: the device keeps its saved state and is bypassed, so the track plays dry until you install the plugin and rescan.

## recording is silent [[desktop]]

- Check that an **input device** is chosen in **settings › audio**. With **none (no recording)**, takes are silent.
- Arm the track and turn its monitoring on: the **input check** at the bottom of the audio tab then shows its level.
- On macOS, if the take is completely silent, the system probably denied microphone access. Ethereal warns you when a take is pure silence. Allow Ethereal in **System Settings › Privacy & Security › Microphone**, then record again.

Recording needs the desktop app. See [recording](/docs/making-music/recording).

## midi keyboards and controllers

- [[desktop]] There's no MIDI device list to set up: every connected MIDI input opens on its own. Each track chooses which ports and channels it listens to in its **IN ▾** panel. If a track ignores your keyboard, check that filter, and that the track is armed or monitoring.
- [[web]] The web version has no hardware MIDI yet. Use the [computer MIDI keyboard](/docs/making-music/computer-midi-keyboard) ([[action:midi.computerKeyboard]]) or the desktop app.
- While the computer MIDI keyboard is on, letter shortcuts play notes instead. Press [[action:midi.computerKeyboard]] to turn it off.
- To control knobs from your controller, see [MIDI learn](/docs/advanced/midi-learn).

## your web projects and storage [[web]]

The web version keeps your projects in this browser's storage, on this computer only. Clearing the site's data, using a private window, or the browser freeing up space can delete them.

- When Ethereal starts in the browser, it asks the browser to keep its storage. If the browser says no, a notice tells you once. **Settings › general › storage** shows the current state (**persistent**, or **may be cleared by the browser when space runs low**), how much space your projects use, and **request again**.
- Browsers decide in their own way: Chrome and Edge are more willing for a site you've installed or bookmarked, Firefox asks you, and Safari has its own rules.
- Whatever the state, keep backups of projects that matter: export them from the project menu, or use the desktop app. See [where your projects live](/docs/welcome/where-your-projects-live) and [moving projects between computers](/docs/basics/moving-projects).

> [!warning]
> Web projects live only in this browser. Export the ones you care about now and then.

## "ethereal is open in another tab" [[web]]

Only one browser tab can work on your projects at a time. A second tab shows **ethereal is open in another tab** with **use here**. **Use here** asks the first tab to save your work and step aside, and that tab then shows **ethereal moved to another tab**. If the first tab doesn't answer within a few seconds, the new tab takes over anyway.

## where next

- [Report an issue](https://github.com/diegoimbert/ethereal/issues) when something here didn't fix it.
- [Audio and MIDI setup](/docs/making-music/audio-and-midi-setup) for the audio settings in detail.
- [FAQ](/docs/reference/faq).
