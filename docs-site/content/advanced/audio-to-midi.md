---
title: audio to midi
order: 60
summary: Turn a melody, chords or a drum loop in an audio clip into MIDI notes on a new track.
platform: all
audience: advanced
covers:
  - midi-editing/convert-audio-to-midi #converting-a-clip
  - midi-editing/audio-to-midi-one-conversion-at-a-time-main-lane-clips-only #one-at-a-time-main-lane-clips-only
---

Audio to MIDI listens to an audio clip and writes the notes it hears into a new MIDI clip. You can then play them with any instrument, fix them in the piano roll, or double a part.

## converting a clip

1. Right-click an audio clip and choose **convert to midi…**.
2. Pick what the clip contains:
   - **melody**: one note at a time (vocals, bass lines, leads);
   - **harmony**: chords and several notes at once (piano, guitar, pads);
   - **drums**: kick, snare and hi-hat hits.
3. Adjust the settings if you need to: the **sensitivity**, and for melody and harmony the **shortest note** and the **pitch range**; for drums, the **kick**, **snare** and **hi-hat** keys the hits go to. **reset settings** puts the defaults back.
4. Optionally turn on **add a poly synth** (or, for drums, **add a drum rack with a kit**) so the new track plays right away.
5. Click **convert**.

The result is a new MIDI track right below the source track, with one clip at the same position and length. The whole conversion is one undo step.

## one at a time, main-lane clips only

- Only one conversion runs at a time. While one runs, **convert to midi…** is greyed out on the other clips.
- **hide** closes the dialog and keeps converting: a progress bar runs along the bottom of the clip. Click it to stop, or choose **stop converting to midi** in the clip's right-click menu.
- Only clips on a track's main lane can be converted, not clips in [take lanes](/docs/making-music/takes-and-comping).
- If a conversion fails, the clip shows it: click the mark for the details.

## where next

- [the piano roll](/docs/making-music/piano-roll): clean up the notes.
- [the drum rack](/docs/making-music/drum-rack)
- [groove, swing and humanize](/docs/advanced/groove)
