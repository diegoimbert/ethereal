---
title: expression and mpe
order: 170
summary: Draw pitch bend, mod wheel, sustain and other controllers in a clip, and use per-note expression with MPE controllers.
platform: all
audience: advanced
covers:
  - midi-editing/expression-lanes-cc-pitch-bend-pressure #choosing-a-lane
  - midi-editing/drawing-and-editing-expression-curves #drawing-and-editing-curves
  - midi-editing/per-note-lanes-draw-over-the-selected-notes-only #per-note-lanes
  - midi-editing/mpe-turn-it-on-per-track-in-the-inspector #mpe
---

Notes are not the only thing a MIDI clip holds. Expression is the movement between and inside notes: pitch bend, the mod wheel, the sustain pedal, pressure. This page shows how to see and draw it under the [piano roll](/docs/making-music/piano-roll), and how to use MPE controllers, which send expression for each note separately.

## choosing a lane

The area under the note grid shows one lane at a time. Pick it with the lane select on its left:

- **notes**: **velocity** (the default; see [velocity](/docs/making-music/piano-roll#the-velocity-lane)), and the per-note lanes **note pressure**, **note pitch** and **note timbre**.
- **clip expression**: **pitch bend**, **channel pressure**, **cc 1** (mod wheel), **cc 11** (expression) and **cc 64** (sustain), any other controller the clip already uses, and **other cc…** to pick any controller number from 0 to 119.
- **envelopes in this clip**: the clip's device envelopes, when it has some. See [automation](/docs/making-music/automation).

A **•** after a lane's name means it already has data in this clip.

Clip expression lanes belong to the clip: they move and loop with it, and their timing lines up with the notes.

## drawing and editing curves

In a clip expression lane:

- **Draw**: drag on the background to draw a curve with the pencil.
- **Move a point**: drag it. It snaps to the grid ([[key:Alt]]: no snapping, and it moves a tenth as far as the mouse, for fine values).
- **Add or delete a point**: double-click the background to add one, or double-click a point to delete it.
- **Clear the lane**: right-click it and choose **clear**. Ethereal asks first.
- **Remove the lane**: click **remove lane**, or right-click and choose **remove lane**. A lane with points asks first; an empty one goes at once.

Each stroke or drag is one step in the [undo history](/docs/basics/undo-and-history).

If you cleared or removed a lane by mistake, undo it with [[action:edit.undo]].

Pitch bend, mod wheel and other controllers you record or [capture](/docs/making-music/capture-midi) also appear here.

## per-note lanes

**Note pressure**, **note pitch** and **note timbre** hold a curve for each note, not one for the whole clip. In these lanes:

- A pencil stroke only writes to the **selected notes**. When no note is selected, it writes to every shown note. Select the notes you want first.
- Right-click and choose **clear** to clear the curves of the selected notes, or of every shown note when none is selected. Ethereal asks first.
- Curve points that go past a note's end are kept but not played.

## mpe

MPE (MIDI polyphonic expression) controllers, such as a Seaboard or a Linnstrument, send pitch, pressure and timbre for each note you play. To record that, turn MPE on for the track:

1. Select the MIDI track.
2. In the inspector, find the **mpe** section and turn on **enabled**.
3. Check the settings against your controller:
   - **zone**: **lower (master 1)** or **upper (master 16)**,
   - **channels**: the number of member channels (up to 15),
   - **note bend**: the per-note pitch bend range (48 semitones by default),
   - **master bend**: the pitch bend range of the master channel.

With MPE on, the per-note pitch, pressure and timbre you record become per-note curves. Pitch curves are also drawn inside the notes in the piano roll, as a line that moves one row per semitone. Those lines are read-only: edit the curves in the **note pitch** lane below.

> [!note]
> If you record from an MPE controller before turning MPE on, the per-note expression isn't recorded as per-note curves. Turn it on first.

## where next

- [Automation](/docs/making-music/automation): change device parameters over time.
- [Recording](/docs/making-music/recording): record a performance with its expression.
- [Instruments](/docs/making-music/instruments): which instruments respond to expression.
