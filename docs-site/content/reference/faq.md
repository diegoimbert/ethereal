---
title: faq
order: 60
summary: Short answers to common questions, including how Ethereal differs from Ableton Live.
platform: all
audience: everyone
---

Quick answers, each with a link to the page that says more. Can't find your question? Search the docs, or [report an issue](https://github.com/diegoimbert/ethereal/issues) and ask.

## the basics

### should i use the desktop app or the web version?

Both make music with the same built-in instruments and effects. The desktop app (macOS, Windows and Linux) adds plugins (CLAP, VST3, AU), recording, hardware MIDI and MIDI learn, and keeps your projects as files on your computer. The web version needs nothing installed, which makes it a good way to try Ethereal. See [what is Ethereal](/docs/welcome/what-is-ethereal).

### where are my projects?

In the desktop app, they're folders on your computer. In the web version, they're inside your browser, on that computer only, and they can be lost if the browser's data is cleared. See [where your projects live](/docs/welcome/where-your-projects-live).

### does it save automatically?

Yes. Ethereal saves as you work, and keeps automatic backups you can go back to. See [projects and saving](/docs/basics/projects) and [versions and backups](/docs/basics/versions-and-backups).

### can i move a project to another computer?

Yes: export it from the project menu as a `.ether` file and open that file on the other computer, in the desktop app or the web version. See [moving projects between computers](/docs/basics/moving-projects).

### does it work on a phone?

A touch version for phones and tablets is in beta. See [Ethereal on your phone](/docs/mobile/mobile-beta).

### does it work offline?

The desktop app makes music without an internet connection. News and the AI assistant need one.

## making music

### why don't my plugins show up?

Plugins need the desktop app. In the desktop app, check **settings › plugins** and rescan. See [plugins](/docs/sounds-and-plugins/plugins) and [troubleshooting](/docs/reference/troubleshooting#plugins-crash-or-fail-to-load).

### can i record?

Yes, in the desktop app: audio from your interface or microphone, and MIDI from your keyboard. The web version can't record. See [recording](/docs/making-music/recording).

### i don't have a midi keyboard. can i still play notes?

Yes: press [[action:midi.computerKeyboard]] to play notes from your computer keyboard. See [the computer MIDI keyboard](/docs/making-music/computer-midi-keyboard). You can also draw notes in [the piano roll](/docs/making-music/piano-roll).

### where are all the shortcuts?

On [keyboard shortcuts](/docs/reference/keyboard-shortcuts). Every command is also in the command palette, [[action:palette.open]], and you can change any shortcut there with **keyboard shortcuts…**.

## files and other apps

### which audio formats can i export?

WAV, FLAC and MP3, as one mix or as stems. See [exporting audio](/docs/import-export/exporting-audio).

## differences from ableton live

Ethereal will feel familiar if you know Live, with a few differences worth knowing:

- **One arrangement, no session view.** There's no clip launcher or scenes: you arrange on a timeline. See [arranging](/docs/making-music/arranging).
- **Copy a clip with [[key:Mod]]-drag.** [[key:Alt]]-drag doesn't copy: it turns snapping off while you drag. See [clips](/docs/making-music/clips).
- **Clicking a track places an insert marker.** Paste, split and insert happen at that marker, not at the playhead. See [playing and moving around](/docs/basics/playing-and-navigating).
- **Space goes back to where you started.** [[action:transport.play]] plays and, pressed again, stops and returns. [[action:transport.stopToSelection]] stops and leaves the playhead where it is. See [playing and moving around](/docs/basics/playing-and-navigating).
- **Live's shortcuts are one switch away.** In **keyboard shortcuts…**, the **Ableton-like** preset adds Live's chords on top of Ethereal's, such as [[action:transport.record]] to record, [[action:transport.loop]] for the loop and [[action:track.addAudio]] / [[action:track.addMidi]] for new audio and MIDI tracks. Nothing is taken away. See [keyboard shortcuts](/docs/reference/keyboard-shortcuts).
- **The computer MIDI keyboard is on [[action:midi.computerKeyboard]]**, like in Live.
- **No separate mixer view.** Volume, pan, mute and solo are on the track headers, and sends are in the inspector. See [mixing](/docs/making-music/mixing).
- **Two warp modes**: repitch and complex. See [audio clips](/docs/making-music/audio-clips).
- **Plugins can run sandboxed** (on macOS and Linux), in their own process, so a crashing plugin doesn't take the app down. See [plugins](/docs/sounds-and-plugins/plugins).

## where next

- [Glossary](/docs/reference/glossary).
- [Troubleshooting](/docs/reference/troubleshooting).
- [Your first beat in 5 minutes](/docs/welcome/first-beat).
