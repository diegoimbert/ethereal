---
title: glossary
order: 20
summary: Short definitions of the words Ethereal uses, each with a link to the page that explains it.
platform: all
audience: everyone
---

These are the words you'll meet in the app and in these docs. Each entry is short and links to the page that explains it properly. You don't need to read this page from top to bottom: use it as a lookup, or search for a word.

## playing and time

- **Playhead**: the line that moves while the song plays. It shows what you hear right now. See [playing and moving around](/docs/basics/playing-and-navigating).
- **Insert marker**: the blinking line a click on a track leaves behind. Paste, split and insert happen there, not at the playhead. See [playing and moving around](/docs/basics/playing-and-navigating).
- **Time selection**: a range of time you select by [[key:Shift]]-dragging over tracks. When there is one, edit keys such as copy and delete act on that time instead of on clips. See [arranging](/docs/making-music/arranging).
- **Loop brace**: the bar above the ruler that marks the part of the song that repeats when looping is on. See [playing and moving around](/docs/basics/playing-and-navigating).
- **Grid and snap**: the beat lines that clips, notes and points jump to while you drag. Hold [[key:Alt]] to place things freely. See [arranging](/docs/making-music/arranging).
- **Marker (locator)**: a named position on the marker lane that you can jump to. See [arranging](/docs/making-music/arranging).
- **Tempo and time signature**: the speed of the song in bpm and how beats are grouped into bars. Both can change during the song. See [tempo, time signature and scale](/docs/basics/tempo-and-time-signature).
- **Count-in**: bars of metronome clicks before recording starts. See [recording](/docs/making-music/recording).
- **Punch in/out**: recording only inside the loop region. See [recording](/docs/making-music/recording).

## tracks and clips

- **Track**: a row in the arrangement. Audio tracks hold recordings and samples, MIDI tracks hold notes and play an instrument. See [tracks](/docs/making-music/tracks).
- **Group track**: a track that holds other tracks and mixes them together, so you can fold, mute or process them as one. See [tracks](/docs/making-music/tracks).
- **Return track and send**: a send passes some of a track's sound to a return track, usually to share one reverb or delay between several tracks. See [mixing](/docs/making-music/mixing).
- **VCA track**: a fader that turns the volume of the tracks assigned to it up or down, without passing their sound through itself. See [tracks](/docs/making-music/tracks).
- **Master track**: where every track ends up before your speakers or an export. See [mixing](/docs/making-music/mixing).
- **Arm**: getting a track ready to record (the ● button on its header). See [recording](/docs/making-music/recording).
- **Monitor**: hearing an input live through a track's devices. See [recording](/docs/making-music/recording).
- **Clip**: a block of audio or notes on a track. See [clips](/docs/making-music/clips).
- **Slip edit**: moving a clip's content inside it while the clip stays in place. See [clips](/docs/making-music/clips).
- **Fade and crossfade**: a short volume ramp at a clip's edge, or between two clips that touch. See [audio clips](/docs/making-music/audio-clips).
- **Warp**: making an audio clip follow the project's tempo. **Repitch** changes speed and pitch together, **complex** keeps the pitch. **Warp markers** pin moments of the audio to beats. See [audio clips](/docs/making-music/audio-clips).
- **Take**: one pass of a recording. Recording over the same place again makes more takes instead of mixing them. See [takes and comping](/docs/making-music/takes-and-comping).
- **Comp (comping)**: building one performance out of the best parts of several takes. See [takes and comping](/docs/making-music/takes-and-comping).
- **Piano roll**: the note editor of a MIDI clip. See [the piano roll](/docs/making-music/piano-roll).
- **Velocity**: how hard a note is played, from 1 to 127. See [the piano roll](/docs/making-music/piano-roll).
- **Quantize**: moving notes onto the grid. See [the piano roll](/docs/making-music/piano-roll).
- **Groove, swing and humanize**: pushing notes off the grid on purpose, in a pattern (swing) or at random (humanize). See [groove, swing and humanize](/docs/advanced/groove).
- **Expression and MPE**: control changes, pitch bend and pressure drawn as curves. MPE gives each note its own curves. See [expression and MPE](/docs/making-music/expression-and-mpe).
- **Capture MIDI**: turning what you just played into a clip, even if you weren't recording. See [capture MIDI](/docs/making-music/capture-midi).

## devices and sound

- **Device**: an instrument or an effect on a track. Built-in devices work in every version of Ethereal; plugins need the desktop app. See [devices and presets](/docs/making-music/devices-and-presets).
- **Device chain**: the devices of a track, in order. Sound flows through them from left to right. See [devices and presets](/docs/making-music/devices-and-presets).
- **Instrument**: a device that turns notes into sound, on a MIDI track. See [instruments](/docs/making-music/instruments).
- **Effect**: a device that changes sound (audio effect) or notes (MIDI effect). See [effects](/docs/making-music/effects).
- **Preset**: saved settings for a device. Factory presets come with Ethereal, your own live in your user library. See [devices and presets](/docs/making-music/devices-and-presets).
- **User library**: your own presets and files, shown in the library next to the factory content. See [the browser](/docs/sounds-and-plugins/browser).
- **Library (browser)**: the panel where you find samples, presets, devices and plugins, and drag them into the project. See [the browser](/docs/sounds-and-plugins/browser).
- **Plugin**: an instrument or effect from another maker (CLAP, VST3 or AU). Desktop only. See [plugins](/docs/sounds-and-plugins/plugins).
- **Sandboxed plugin**: a plugin that runs in its own process, so a crash doesn't stop the app. See [plugins](/docs/sounds-and-plugins/plugins).
- **Drum rack**: an instrument with one pad per sound, each with its own chain. See [the drum rack](/docs/making-music/drum-rack).
- **Choke group**: pads that cut each other off, like an open and a closed hi-hat. See [the drum rack](/docs/making-music/drum-rack).
- **Rack**: a device that holds several chains side by side. See [racks and macros](/docs/making-music/racks-and-macros).
- **Chain**: one path through a rack, with its own devices. See [racks and macros](/docs/making-music/racks-and-macros).
- **Macro**: one knob on a rack that turns several parameters inside it at once. See [racks and macros](/docs/making-music/racks-and-macros).
- **Modulator**: a source such as an LFO or an envelope that moves parameters on its own. See [modulation and sidechain](/docs/making-music/modulation-and-sidechain).
- **Sidechain**: letting one track's sound drive an effect on another, for example a kick ducking a bass. See [modulation and sidechain](/docs/making-music/modulation-and-sidechain).
- **Automation**: parameter changes drawn over time in a lane. See [automation](/docs/making-music/automation).
- **Clip envelope**: automation that lives inside a clip and moves with it. See [automation](/docs/making-music/automation).
- **MIDI learn (MIDI mapping)**: linking a knob on your MIDI controller to a control in Ethereal. See [MIDI learn](/docs/advanced/midi-learn).
- **Computer MIDI keyboard**: playing notes from your computer's keyboard. See [the computer MIDI keyboard](/docs/making-music/computer-midi-keyboard).

## rendering

- **Freeze**: rendering a track to audio so its devices stop using the cpu. A frozen track is locked until you unfreeze it. See [freeze, flatten and bounce](/docs/advanced/freeze-and-bounce).
- **Flatten**: replacing a frozen track's clips and devices with one audio clip of the render. Only undo brings them back. See [freeze, flatten and bounce](/docs/advanced/freeze-and-bounce).
- **Bounce and consolidate**: rendering a part of the song to a new audio clip. See [freeze, flatten and bounce](/docs/advanced/freeze-and-bounce).
- **Stems**: one audio file per chosen track, each exported on its own. See [exporting audio](/docs/import-export/exporting-audio).
- **Dropout (xrun)**: a click or gap in the sound when the computer can't fill the audio buffer in time. See [troubleshooting](/docs/reference/troubleshooting).
- **Buffer size and latency**: how much audio is computed at once. Smaller buffers mean less delay but more risk of dropouts. See [audio and MIDI setup](/docs/making-music/audio-and-midi-setup).

## projects

- **Project**: one song, with its tracks, clips, devices and media. See [projects and saving](/docs/basics/projects).
- **Draft**: the project you get when you close the welcome screen. If you leave it untouched, it is thrown away. See [projects and saving](/docs/basics/projects).
- **Template**: a project that new projects start from. See [projects and saving](/docs/basics/projects).
- **Versions**: automatic backups of a project that you can go back to. A **snapshot** is a version you save on purpose, with a name. See [versions and backups](/docs/basics/versions-and-backups).
- **Checkpoint**: a named step in the undo history. See [undo and history](/docs/basics/undo-and-history).
- **.ether file**: a whole project in one file, to move it to another computer or keep a backup. See [moving projects between computers](/docs/basics/moving-projects).
- **Media**: the audio files a project uses. See [moving projects between computers](/docs/basics/moving-projects).
- **Safe mode**: opening a project with every plugin turned off, to get past a plugin that crashes. See [safe mode](/docs/advanced/safe-mode).
- **Command palette**: the search box for every command, opened with [[action:palette.open]]. See [the command palette and settings](/docs/basics/command-palette-and-settings).

## sharing

- **Live session**: working on one project together, in real time. See [live sessions](/docs/sharing/live-sessions).
- **Host**: the person who shared the project. Their computer keeps the session together. See [live sessions](/docs/sharing/live-sessions).
- **Invite link**: the link that lets someone join. A **can edit** link lets them change the project; a **can listen** link only lets them listen. See [sharing your music](/docs/sharing/overview).
- **Offline copy**: the copy of a shared project you keep working on while the host is offline. Your edits sync when the host is back. See [live sessions](/docs/sharing/live-sessions).
- **Local copy**: a separate project named "… (local copy)". It is made when changes you made offline to a shared project can't be merged back into the session. See [live sessions](/docs/sharing/live-sessions).
- **Play with**: choosing whose computer plays the sound in a session. See [listening on a friend's computer](/docs/sharing/listening-together).
- **Static link (snapshot)**: a read-only copy of a project published on the web, for people to listen to. See [static share links](/docs/sharing/static-links).
- **Share viewer**: the read-only page that opens a static link. See [the share viewer](/docs/sharing/share-viewer).

## the app

- **Engine**: the part of Ethereal that makes the sound. In the desktop app it runs on your computer; in the web app it runs inside your browser tab. See [what is Ethereal](/docs/welcome/what-is-ethereal).
- **Engine server**: an engine running on another machine (`ether-server`) that this window connects to. See [the engine server](/docs/advanced/engine-server).
- **MCP**: the way outside AI agents can control Ethereal. See [AI agents (MCP)](/docs/advanced/mcp).
- **Inspector**: the panel that shows the settings of what you selected. See [a tour of the screen](/docs/basics/the-screen).
- **Editor drawer**: the panel at the bottom with the clip editors (piano roll, warp, automation). [[action:view.editor]] shows or hides it. See [a tour of the screen](/docs/basics/the-screen).
- **Mobile beta**: the touch version of Ethereal for phones and tablets. See [Ethereal on your phone](/docs/mobile/mobile-beta).

## where next

- [Troubleshooting](/docs/reference/troubleshooting) when something doesn't work.
- [FAQ](/docs/reference/faq) for quick answers.
- [Keyboard shortcuts](/docs/reference/keyboard-shortcuts).
