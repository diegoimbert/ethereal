---
title: faq
order: 60
summary: Short answers to common questions, including how Ethereal differs from Ableton Live.
platform: all
audience: everyone
---

Quick answers, each with a link to the page that says more. Can't find your question? Search the docs, or [report an issue](/docs/reference/reporting-issues) and ask.

## the basics

### should i use the desktop app or the web version?

Both make music with the same built-in instruments and effects. The desktop app (macOS, Windows and Linux) adds plugins (CLAP, VST3, AU), recording, hardware MIDI and MIDI learn, and keeps your projects as files on your computer. The web version needs nothing installed, which makes it a good way to try Ethereal or to join a friend's session. See [what is Ethereal](/docs/welcome/what-is-ethereal).

### where are my projects?

In the desktop app, they're folders on your computer. In the web version, they're inside your browser, on that computer only, and they can be lost if the browser's data is cleared. See [where your projects live](/docs/welcome/where-your-projects-live).

### does it save automatically?

Yes. Ethereal saves as you work, and keeps automatic backups you can go back to. See [projects and saving](/docs/basics/projects) and [versions and backups](/docs/basics/versions-and-backups).

### can i move a project to another computer?

Yes: export it from the project menu as a `.ether` file and open that file on the other computer, in the desktop app or the web version. See [moving projects between computers](/docs/basics/moving-projects).

### does it work on a phone?

A touch version for phones and tablets is in beta. See [Ethereal on your phone](/docs/mobile/mobile-beta).

### does it work offline?

The desktop app makes music without an internet connection. Sharing, joining sessions, news, update checks, issue reports and the AI assistant need one.

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

### can i open my ableton live or fl studio projects?

Yes, in beta: Ableton Live sets (`.als`), FL Studio projects (`.flp`), DAWproject files and REAPER projects open as a new Ethereal project. A report lists what couldn't be brought over, and helps you replace missing plugins and samples. See [projects from other DAWs](/docs/import-export/other-daws).

### can i send my project to another daw?

Yes: export to DAWproject, Ableton Live Set (`.als`) or FL Studio project (`.flp`). See [exporting to other DAWs](/docs/import-export/exporting-to-other-daws).

### which audio formats can i export?

WAV, FLAC and MP3, as one mix or as stems. See [exporting audio](/docs/import-export/exporting-audio).

## sharing

### how do i make music with a friend?

Press **share** and send them a link. They can join from the desktop app or straight from their browser, without an account. Everyone plays the music on their own computer. See [sharing your music](/docs/sharing/overview).

### if i undo in a session, do i undo my friend's work?

No. Undo only undoes your own changes. See [undo and history](/docs/basics/undo-and-history).

### is my shared project private?

A live session travels encrypted between your computers, and the people in it can see each other's IP address. Anyone with a link can use it, so share links only with people you trust, and reset a link you've sent too widely. See [live sessions](/docs/sharing/live-sessions) and [privacy and telemetry](/docs/reference/privacy-and-telemetry).

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
- **Plugins can run sandboxed**, in their own process, so a crashing plugin doesn't take the app down. See [plugins](/docs/sounds-and-plugins/plugins).
- **Live sessions are built in.** Share a link and work on the same project together. See [sharing your music](/docs/sharing/overview).
- **Projects from Live open in Ethereal**, and Ethereal can export `.als` files, in beta. See [projects from other DAWs](/docs/import-export/other-daws).

## where next

- [Glossary](/docs/reference/glossary).
- [Troubleshooting](/docs/reference/troubleshooting).
- [Your first beat in 5 minutes](/docs/welcome/first-beat).
