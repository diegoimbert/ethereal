---
title: projects from other daws
order: 20
summary: Open Ableton Live, FL Studio, REAPER and DAWproject files as Ethereal projects, read the import report and resolve what's missing.
platform: all
audience: everyone
covers:
  - import-daw/which-daw-files-open #which-files-open
  - import-daw/where-to-start-an-import #starting-an-import
  - import-daw/an-import-makes-a-new-project #an-import-makes-a-new-project
  - import-daw/import-is-beta #importing-is-in-beta
  - import-daw/progress-and-cancel #while-it-imports
  - import-daw/reading-the-import-report #the-import-report
  - import-daw/right-click-a-report-line-to-resolve-it #the-import-report
  - import-daw/nothing-resolvable-is-thrown-away #nothing-is-thrown-away
  - import-daw/resolve-missing-devices-samples #resolving-missing-items
  - import-daw/resolve-later-and-getting-back-to-it #resolving-later
  - import-daw/show-when-a-project-opens-switch #resolving-later
  - import-daw/replacing-a-missing-device-carries-values-over #replacing-a-device-or-plugin
  - import-daw/plugin-state-kept-only-for-the-same-plugin #replacing-a-device-or-plugin
  - import-daw/ai-suggestions-for-missing-plugins-optional #ai-suggestions
  - import-daw/finding-missing-audio-files #finding-missing-audio-files
  - import-daw/on-the-web-referenced-audio-comes-in-missing #audio-files-on-the-web
  - import-daw/ableton-live-what-converts #ableton-live
  - import-daw/ableton-live-what-is-lost-or-approximated #ableton-live
  - import-daw/fl-studio-what-converts-and-what-doesn-t #fl-studio
  - import-daw/dawproject-reaper-per-app-gaps #dawproject-and-reaper
  - import-daw/size-limits-on-imports #size-limits
---

Started a song in another DAW? Ethereal can open it as a new Ethereal project. This page covers which files open, how to read the report that follows, and how to fix what didn't come across. To send a project the other way, see [exporting to other DAWs](/docs/import-export/exporting-to-other-daws).

## which files open

| Format | Files | Made by |
|---|---|---|
| Ableton Live sets | `.als` | Ableton Live |
| FL Studio projects | `.flp` | FL Studio |
| REAPER projects | `.rpp`, `.rpp-bak` | REAPER |
| DAWproject | `.dawproject` | Bitwig Studio, Studio One, Cubase, Nuendo, Cubasis, VST Live, n-Track Studio, Waveform |

DAWproject is an open exchange format. If your DAW can export it (for example **File › Export DAWproject** in Bitwig Studio, or **File › Convert To › DAWproject File** in Studio One), that's usually the most faithful route. Logic Pro and Pro Tools can't write any of these formats.

## starting an import

Any of these works:

- In the project menu (top left), **import from another daw**, then pick the DAW. Each choice only shows that DAW's files.
- On the welcome screen, **import from another daw…**.
- In the command palette ([[action:palette.open]]), **import from another daw…**.
- Drop the file anywhere on Ethereal's window.

### an import makes a new project

Importing never changes the original file and never merges into the project you have open. Ethereal creates a new project from the file and switches to it when it's done. If you're in a [live session](/docs/sharing/live-sessions), Ethereal first asks you to leave it (**leave & import**), from the menu or when you drop the file on the window.

## importing is in beta

Converting from other DAWs is best effort: some things may be missing or imported wrong. Ethereal says so while it imports and in the report. If an import goes badly, click **send diagnostics…** in the report to tell us.

## while it imports

A dialog shows the current stage and a progress bar. **Cancel** stops the import and removes the half-made project.

## the import report

When the import finishes, the new project opens and a report lists what came in (tracks, clips, notes and so on), then up to three sections:

- **missing: kept to resolve**: plugins that aren't installed, devices Ethereal has no equivalent for, audio files that weren't found. They're in the project, waiting for you.
- **imported approximately**: things that came across close to, but not exactly like, the original.
- **not imported**: things Ethereal couldn't bring over.

Each line is tagged with its area (tracks, devices, plugins, audio files…) and a count. Right-click a plugin, audio file or missing device line and choose **resolve…** to fix that one item straight away. The report keeps the first 500 distinct lines, then says how many more there are.

### nothing is thrown away

Anything that could be fixed later stays in the project:

- A **plugin that isn't installed** keeps its saved sound, its values and its automation. It plays bypassed until you install it or replace it.
- A **device of the other DAW** that Ethereal has no equivalent for (Live's Hybrid Reverb, Max for Live devices, Bitwig's Polymer…) stays in the chain as a **missing device**, with its values and automation. Audio passes through it unchanged, and a missing instrument is silent.
- An **audio file that can't be found** stays as a missing reference that its clips point to.

## resolving missing items

Click **done** in the report and Ethereal opens **resolve missing items** (the report also has **resolve missing devices** and **resolve missing samples** buttons). It lists every missing plugin, device and audio file, each with:

- a proposed replacement,
- how sure the match is: **same plugin id**, **same name**, **same family** or **similar name**, with a percentage,
- and why it was proposed.

For each item you can switch the proposal on, pick another replacement from the list, or leave it. Click **apply** to make the changes. All of them together are one undo step. **Accept all** applies every proposal at once.

### replacing a device or plugin

You can replace a missing device with a built-in device of the same kind or with any installed plugin. Its values, automation, modulation and MIDI mappings move to the parameters with the same name: "DryWet", "Dry/Wet" and "Mix" count as the same. Anything that has no match is listed in a notification after you apply.

For plugins:

- Replaced by **the same plugin** in another version or format, the plugin keeps its saved sound ("plugin state carried over").
- Replaced by **a different plugin**, only parameter values that match by name (or by position, for the same plugin) survive. The dialog lists what will be lost.

### ai suggestions

If you've set up an AI provider in the [AI assistant](/docs/advanced/ai-assistant), the dialog can ask it about plugins with no sure match. Only plugin details are sent: names, vendors, formats, ids, versions and saved parameter names. **show what will be sent** shows you exactly what goes out, and Ethereal asks for your consent once. Suggestions are marked **AI suggestion** and only apply when you switch them on.

### finding missing audio files

Ethereal looks for a missing audio file by name, then checks the sample rate, channels and length (and the file's content, when it's known). It searches the imported project's folder, the `Samples` folders next to it and above it, and your library. On the desktop, **search folder…** searches a folder you choose. You can also pick the file yourself with **choose file…**.

### resolving later

Every item can wait. Click **resolve later** and the project keeps the missing items as they are. To come back to them:

- the **n missing** badge in the top bar,
- **resolve missing devices and samples…** in the project menu or the command palette,
- the **resolve…** button on a missing device,
- right-click a missing clip or device and choose **resolve…**.

The dialog also opens by itself when you open a project that has missing items. Turn off **show when a project opens** in the dialog to stop that.

## audio files on the web [[web]]

A REAPER or DAWproject file can point to audio files elsewhere on your disk instead of holding them. The desktop app finds them in the folders next to the project. The browser can't read your disk, so those clips arrive as missing references: relink them in the resolver with **choose file…**. Audio stored inside the file itself (DAWproject can embed it) always comes along.

## what converts from each daw

### ableton live

What comes over: tracks, groups, return tracks, the mixer and sends, arrangement clips with warping, fades, gain and transposition, take lanes, track and clip automation of the devices Ethereal maps, and most of Live's stock devices: EQ Eight, Compressor, Gate, Delay, Utility and many more, plus Instrument, Audio Effect, MIDI Effect and Drum Racks, Simpler and Sampler, and VST2, VST3 and AU plugins with their state. A frozen track comes in as its frozen audio on a new track, with the original kept and muted.

What's lost or approximated:

- **Session View clips** are laid out scene by scene (with a marker for each scene) only when the arrangement is empty. Otherwise they're left out.
- **Note chance and velocity range** (Live 11+) are ignored: notes play at their own velocity.
- **Clip gain and transposition envelopes**, and envelopes unlinked from their clip, are dropped.
- **Arpeggiator, Chord, Scale, Note Length and Random** come in as Ethereal's devices with default settings.
- **Rack macros** come in without their targets.
- **A Drum Rack inside a pad** is flattened into the outer rack.
- **Tones and Texture warp modes** become Ethereal's stretch.
- **Native devices with no equivalent** (Hybrid Reverb, Operator, Max for Live…) are kept as missing devices to resolve.

### fl studio

Each channel that plays something becomes a track. Pattern clips on the playlist become MIDI clips, and the mixer inserts the channels play into become group tracks (or return tracks, when they receive sends). What's lost or approximated:

- Only the **current arrangement** is imported.
- When the **playlist is empty** (a project made only in the channel rack and piano roll), the patterns are laid out one after another in pattern order, from bar 1, each as long as its notes rounded up to the bar, with a marker for each pattern. Each channel gets its own track, and each pattern a clip with its name and color on the channels it plays.
- **Automation clips** come in only when they automate a channel's volume.
- FL's **time-stretch and pitch** settings on audio are ignored: audio plays at its own speed.
- FL's **native effects** become the nearest built-in device with default settings. Native plugins with no equivalent are kept as missing devices.
- **Volume and fader curves** are approximated.

### dawproject and reaper

Both formats come in well: tracks, groups, returns and sends, MIDI and audio clips, tempo and time signature changes, markers, plugins with their state and most automation. What's lost or approximated:

- **Fade shapes** become linear, **send pan** and **master solo** are dropped.
- **VCA tracks** become groups. **Clip launcher scenes** and **video** are left out.
- **Waveform** doesn't export automation or sends, so they're not in the file.
- **Studio One**'s own effects arrive only as generic EQ, compressor, gate and limiter.
- **Bitwig**'s own devices (Polymer, Grid…) are kept as missing devices.
- From **REAPER**: automation of VST3 and CLAP parameters, JS and LV2 effects, AU plugins and regions aren't imported.

## size limits

Very large projects are cut short, and the report says so: an import stops at 4 million notes or 1 million clips, and a project can't exceed 512 MB. One audio file inside a DAWproject can be up to 2 GB on the desktop and 1 GB in the browser.

## where next

- [Plugins](/docs/sounds-and-plugins/plugins): installing and rescanning plugins.
- [Exporting to other DAWs](/docs/import-export/exporting-to-other-daws): send a project back.
- [Importing audio](/docs/import-export/importing-audio): just the audio files.
