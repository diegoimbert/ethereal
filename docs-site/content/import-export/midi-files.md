---
title: midi files
order: 15
summary: Turn MIDI files into MIDI clips, and save your MIDI clips as MIDI files for other apps.
platform: all
audience: everyone
covers:
---

A MIDI file (`.mid` or `.midi`) holds notes, not sound: a melody, a drum pattern or a whole song's parts, made in another app or downloaded. This page shows how to bring one into a project as MIDI clips, and how to save your own clips as a MIDI file. You need an open project.

## importing a midi file

Drag a MIDI file onto the arrangement, from [the browser](/docs/sounds-and-plugins/browser#midi-files) or from your computer's file manager:

- **on a MIDI track**: the file's first part becomes a clip on that track, where you drop it. Each other part of the file gets a new MIDI track right after it.
- **below the tracks**: each part of the file gets a new MIDI track, all starting where you drop it.
- **on an audio track**: a file from the browser can't go there. A file from your computer goes on new MIDI tracks, as if dropped below the tracks.

Each part becomes one clip. Its notes keep their velocity and length, and the clip is as long as the whole file, rounded up to whole bars. New tracks get the built-in **synth**, named after the part in the file (or after the file and its MIDI channel when the part has no name). A file that keeps all its parts in one track (a "type 0" file) is split by MIDI channel, so the drums and the bass still get their own tracks.

The whole import is one undo step: [[action:edit.undo]] removes every track and clip it added. The new clips are selected, ready to move.

### tempo

Notes are placed by beats, so they stay on the grid. In a project that has no clips yet, the file's tempo and time signature (and their changes) become the project's, so the song plays at the speed it was written at. In a project that already has clips, the project keeps its tempo and the notes follow it.

## exporting midi

To save MIDI clips as a standard MIDI file, which any DAW or notation app can open:

- **a clip, or several**: select MIDI clips, right-click one and choose **export MIDI…**. The file starts at the first selected clip.
- **a whole track**: right-click a MIDI track's header and choose **export MIDI…**. Every MIDI clip of the track goes in, and the file starts at the beginning of the project, so the clips keep their place. With several tracks selected, each one becomes a part of the file. On a track without MIDI clips the entry is greyed out.

Clips are written as they play: a looped clip is repeated over its length, and a trimmed clip only gives the notes you hear. Muted notes are left out. The file also carries the project's tempo and time signatures.

On the desktop, a save dialog asks where to put the file. In the browser, the file is downloaded.

## where next

- [The piano roll](/docs/making-music/piano-roll): edit the imported notes.
- [Importing audio](/docs/import-export/importing-audio): bring in audio files the same way.
