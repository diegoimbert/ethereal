---
title: groove, swing and humanize
order: 50
summary: Swing the whole song at playback, write swing into a clip, and loosen up timing and velocity.
platform: all
audience: advanced
covers:
  - midi-editing/quantize-and-humanize-settings-are-not-saved #the-settings-are-remembered
  - midi-editing/humanize-randomizes-timing-and-velocity #humanize
  - midi-editing/project-swing-vs-quantize-swing #project-swing-vs-quantize-swing
---

Notes drawn on a grid can sound stiff. Ethereal has three tools to add feel to MIDI: the project's playback swing, the swing inside **quantize…**, and **humanize…**. Quantizing itself is explained in [the piano roll](/docs/making-music/piano-roll).

## project swing vs quantize swing

There are two kinds of swing, and they work differently:

- **Project swing** delays the off-beat notes of every MIDI clip while the song plays. No note moves: turn it back to 0% and everything is straight again. Set it in the **groove** row of the inspector when a MIDI clip is selected: an amount in percent and a grid (**1/8** or **1/16**). It's saved with the project.
- **Swing in quantize…** writes swing into the notes of one clip: the off-beat notes really move. Set **swing** in the piano roll's **quantize…** popover, then quantize the notes ([[action:pianoRoll.quantize]]).

Use project swing to try a feel on the whole song, and quantize swing when one part needs its own.

## humanize

**humanize…** in the piano roll's toolbar (or **humanize** in a note's right-click menu) shifts notes randomly in time and varies their velocity:

- **timing (±1/16)**: at 100%, a note can move up to a sixteenth note earlier or later.
- **velocity**: how much the velocities vary.

It applies to the selected notes, or to the whole clip when nothing is selected. Each run gives a different random result, and each run is one undo step.

## the settings are remembered

The values you set in the **quantize…** and **humanize…** popovers are remembered on this computer, also after a restart, and [[action:pianoRoll.quantize]] uses them. They belong to the app, not to a project: every project uses the same ones. Project swing, on the other hand, is saved with the project.

## where next

- [the piano roll](/docs/making-music/piano-roll): drawing notes and quantizing.
- [audio to midi](/docs/advanced/audio-to-midi): turn a recording into notes.
