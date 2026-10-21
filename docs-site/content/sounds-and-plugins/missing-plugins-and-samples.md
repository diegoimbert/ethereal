---
title: missing plugins and samples
order: 30
summary: Replace the plugins, devices and audio files a project uses that aren't on this computer, now or later.
platform: all
audience: everyone
covers:
  - import-daw/resolve-missing-devices-samples #resolving-missing-items
  - import-daw/resolve-later-and-getting-back-to-it #resolving-later
  - import-daw/show-when-a-project-opens-switch #resolving-later
  - import-daw/replacing-a-missing-device-carries-values-over #replacing-a-device-or-plugin
  - import-daw/plugin-state-kept-only-for-the-same-plugin #replacing-a-device-or-plugin
  - import-daw/ai-suggestions-for-missing-plugins-optional #ai-suggestions
  - import-daw/finding-missing-audio-files #finding-missing-audio-files
---

A project can use things that aren't on the computer you open it on: a plugin you haven't installed, or an audio file that was moved. Ethereal never throws them away. A missing plugin stays in its chain, bypassed, with its saved sound, and a missing audio file keeps its clip. This page shows how to replace them, or leave them for later. To fix a single missing sample, [relinking](/docs/basics/moving-projects#missing-samples) is often quicker.

## resolving missing items

**resolve missing items** lists every missing plugin, device and audio file, each with:

- a proposed replacement,
- how sure the match is: **same plugin id**, **same name**, **same family** or **similar name**, with a percentage,
- and why it was proposed.

For each item you can switch the proposal on, pick another replacement from the list, or leave it. Click **apply** to make the changes. All of them together are one undo step. **accept all** applies every proposal at once.

### replacing a device or plugin

You can replace a missing device with a built-in device of the same kind or with any installed plugin. Its values, automation, modulation and MIDI mappings move to the parameters with the same name: "DryWet", "Dry/Wet" and "Mix" count as the same. Anything that has no match is listed in a notification after you apply.

For plugins:

- Replaced by **the same plugin** in another version or format, the plugin keeps its saved sound ("plugin state carried over").
- Replaced by **a different plugin**, only parameter values that match by name (or by position, for the same plugin) survive. The dialog lists what will be lost.

To swap a device that isn't missing, right-click its header and choose **replace device…** (see [plugins](/docs/sounds-and-plugins/plugins#missing-plugins)).

### ai suggestions

If you've set up an AI provider in the [AI assistant](/docs/advanced/ai-assistant), the dialog can ask it about plugins with no sure match. Only plugin details are sent: names, vendors, formats, ids, versions and saved parameter names. **show what will be sent** shows you exactly what goes out, and Ethereal asks for your consent once. Suggestions are marked **AI suggestion** and only apply when you switch them on.

### finding missing audio files

Ethereal looks for a missing audio file by name, then checks the sample rate, channels and length (and the file's content, when it's known). On the desktop, **search folder…** searches a folder you choose. You can also pick the file yourself with **choose file…**.

## resolving later

Every item can wait. Click **resolve later** and the project keeps the missing items as they are. To come back to them:

- the **n missing** badge in the status bar,
- **resolve missing devices and samples…** in the project menu or the command palette,
- the **resolve…** button on a missing device,
- right-click a missing clip or device and choose **resolve…**.

The dialog also opens by itself when you open a project that has missing items. Turn off **show when a project opens** in the dialog to stop that.

## where next

- [Plugins](/docs/sounds-and-plugins/plugins): installing and rescanning plugins.
- [Moving projects between computers](/docs/basics/moving-projects): take a project and its samples with you.
- [Safe mode](/docs/advanced/safe-mode): open a project without its plugins.
