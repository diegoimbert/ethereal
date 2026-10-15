---
title: capture midi
order: 160
summary: Turn what you just played into a clip, even if you weren't recording.
platform: all
audience: everyone
covers:
  - recording/capture-midi-rescue-what-you-just-played #rescue-what-you-just-played
  - recording/capture-while-stopped-sets-the-tempo-empty-projects-only #played-while-stopped
  - recording/capture-keeps-only-the-latest-take-then-empties #what-capture-keeps
  - recording/capture-respects-the-target-track-s-midi-filter #when-capture-finds-nothing
---

You played something great while you were just noodling, and you weren't recording. Capture MIDI gets it back. Ethereal always listens to the MIDI you play, so you can turn it into a clip afterwards.

It works with a MIDI controller on the desktop app, and with the [computer MIDI keyboard](/docs/making-music/computer-midi-keyboard) everywhere, the web included.

## rescue what you just played

Click the capture button in the transport bar. Its tooltip reads "capture MIDI: turn what you just played into a clip". The button stays dim until you have played something. It is also in the command palette ([[action:palette.open]]) as **capture MIDI**.

You don't need to arm a track or press record. The new clip goes to:

1. the selected track, if it is a MIDI track,
2. else the first armed MIDI track,
3. else the first MIDI track,
4. else a new MIDI track.

The new clip is selected. The whole capture is one step in the [undo history](/docs/basics/undo-and-history).

## played while the song was playing

Notes you played while the transport was running keep their positions in the song. The clip spans the bars you played. If you played over a loop, the passes are merged inside the loop.

## played while stopped

Notes played while the song was stopped become a clip at the playhead. Capture takes your last phrase: a pause of more than 6 seconds starts a new phrase, and anything before it is left out. The clip length is rounded up to whole bars.

If the project is empty (no clips at all), capture also guesses the tempo from what you played, between 60 and 180 bpm, taking your first note as the first downbeat. It then sets the project tempo and puts the loop around the new clip. In a project that already has clips, capture never changes the tempo.

## what capture keeps

- Every play or stop starts a new capture take. Capture uses only the latest take that has notes, not something from several play/stop cycles ago.
- After a capture the buffer is emptied, and the button goes dim until you play again.
- It holds up to 10 minutes of playing.
- It is cleared when you switch projects. It is never saved, and it is not shared in a [live session](/docs/sharing/live-sessions).

Pitch bend, mod wheel and other controllers you played are captured too, as [expression lanes](/docs/making-music/expression-and-mpe) on the clip.

## when capture finds nothing

Capture only keeps the MIDI that the target track would accept: its input port and channel filter (see [MIDI input filters](/docs/making-music/recording#midi-input-filters)). If the selected track listens to another port or channel than the one you played on, capture says nothing was played. Select another track, or set its input to **all ports** and **all channels**.

## where next

- [The piano roll](/docs/making-music/piano-roll): clean up the captured notes.
- [Recording](/docs/making-music/recording): record on purpose, with count-in and takes.
