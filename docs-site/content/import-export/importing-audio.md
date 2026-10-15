---
title: importing audio
order: 10
summary: Bring audio files from your computer into a project, with the import shortcut or by dropping them.
platform: all
audience: beginner
covers:
  - import-daw/import-audio-files-i-ctrl-i #import-audio
  - import-daw/dropping-audio-vs-project-files #dropping-files-on-the-window
  - import-daw/audio-import-limits-and-errors #limits-and-errors
---

This page shows how to get audio files from your computer into the open project: loops, recordings, stems from a friend. To use sounds from your sample library, drag them from [the browser](/docs/sounds-and-plugins/browser) instead.

## import audio…

Press [[action:file.import]], or choose **import audio…** in the command palette or in the arrangement's right-click menu. Pick one or more files. Ethereal reads WAV, AIFF, FLAC, MP3, OGG and M4A/AAC files.

Where the files land:

- **with an audio track selected**: on that track, starting at the insert marker (where you last clicked in the arrangement, not the playhead), one after the other.
- **otherwise**: each file goes on a new audio track, all starting at the insert marker.

The whole import is one undo step, so [[action:edit.undo]] removes every file it added.

> [!note]
> On the desktop, an imported file is used from where it is on your disk, not copied into the project. If you move or delete it, the clip shows as missing. To take a project to another computer with its audio, use **export project…** with the media packed in. See [moving projects](/docs/basics/moving-projects).

## dropping files on the window

You can also drag files from your computer's file manager:

- **audio files on an audio track**: the clips are placed where you drop them, one after the other.
- **audio files below the tracks**: each one gets a new audio track.
- **audio files on the browser**: they're added to the project without placing a clip.
- **MIDI files** (`.mid`, `.midi`) on a MIDI track or below the tracks: they become MIDI clips. See [midi files](/docs/import-export/midi-files).
- **a project from another DAW** (`.als`, `.flp`, `.rpp`, `.dawproject`), dropped anywhere: it's imported as a new project. See [projects from other DAWs](/docs/import-export/other-daws).
- **an Ethereal project** (`.ether`), dropped anywhere: it's imported and opened as a new project.

## limits and errors

Ethereal checks each file before reading it. Files over 1 GB, empty files and files of other types are refused, with the reason ("use WAV, AIFF, FLAC, MP3 or OGG", "too large", "the file is empty").

Each file being imported shows a row with its progress at the bottom of the window. If one file fails, the others still import, and the failed one stays in the list with its error until you dismiss it. You can cancel a file while it's still being read. Once its audio is in the project, the import can no longer be cancelled: use undo instead.

In a [live session](/docs/sharing/live-sessions), imported audio is sent to everyone in the session.

## where next

- [Clips](/docs/making-music/clips): move, trim and loop your audio.
- [Audio clips](/docs/making-music/audio-clips): fades, warp and reverse.
- [Projects from other DAWs](/docs/import-export/other-daws): bring a whole song.
