---
title: the drum rack
order: 80
summary: Build a drum kit from samples, one sound per pad, with choke groups and effects per pad.
platform: all
audience: beginner
covers:
  - racks-modulation/drum-rack-pads #pads
  - racks-modulation/choke-groups #choke-groups
  - racks-modulation/pad-solo-isn-t-saved-or-undoable #mute-and-solo-pads
  - racks-modulation/slice-a-sample-into-a-drum-rack #slicing-a-sample-to-pads
---

A drum rack is an instrument that plays a different sound on each note: one kick on C1, a snare on D1, and so on. Each sound lives on a **pad**, and each pad has its own volume, pan and chain of devices. You need a MIDI track: a drum rack is an instrument, so it goes in the track's instrument slot (see [devices and presets](/docs/making-music/devices-and-presets#adding-a-device)).

## making a kit

1. On a MIDI track, add **drum rack** from the instrument library. Its window opens.
2. Open the [browser](/docs/sounds-and-plugins/browser) and drag a sample onto an empty pad. The pad is created with a sampler playing that sample.
3. Repeat for each sound. Dropping a sample on a pad that already has one replaces the sample.
4. Write the beat: a MIDI clip on a drum rack track opens the [beat editor](/docs/making-music/piano-roll#drum-rack-tracks-open-the-beat-editor), where each row is a pad.

## slicing a sample to pads

A drum loop can become a kit: Ethereal cuts it at its transients (the start of each hit) and puts each slice on its own pad, with a sampler playing just that slice.

- In a drum rack's window, click **slice to pads** and pick a sample, or drag a sample from the [browser](/docs/sounds-and-plugins/browser) onto that button. The slices go on the keys right above the rack's pads (from C1 on an empty rack).
- In the browser, right-click a sample: **slice to new drum rack** makes a new MIDI track with a drum rack of its slices; **slice to the selected drum rack** adds them to the drum rack of the selected track.
- Right-click an audio clip and choose **slice to drum rack**: its sample is sliced into a drum rack on a new MIDI track, right below the clip's track.
- A [sampler in slice mode](/docs/making-music/instruments#slice-mode) has a **slice to drum rack** button too, which uses the slices you set by hand.

Each of these is one undo step.

## pads

The window shows 16 pads (4 × 4), one **bank**. The **bank** menu shows other groups of 16 notes. The first bank starts on **C1** (note 36), where most drum kits begin. Ethereal names notes with C3 = 60, like Ableton Live.

- **Click** a pad to select it. Its settings and its chain appear under the grid.
- **Press and hold** a pad to hear it. The sound goes through the pad's chain exactly like playback, whether the track is armed or not and whether the song is playing or not. The play button on a pad does the same.
- **Double-click** a pad's name to rename it. An empty name brings back the note name.
- **Right-click** a pad for **rename**, **mute**, **solo**, a quick choke group, and **delete pad**.

The selected pad's settings:

- **volume**, **pan** and **tune** (tune transposes the pad's sampler, in semitones);
- **choke**: its choke group;
- its **chain**: add effects to this pad only (a reverb on the snare, a saturator on the kick). A pad's chain works like a track's, with its own instrument slot.

An empty pad has an instrument slot too: adding an instrument there creates the pad with it, so a pad can play a synth instead of a sample.

## choke groups

Pads in the same **choke group** (1 to 16) cut each other off: when one starts, the others in its group stop. The classic use is the hi-hat: put the closed and the open hi-hat in the same group, so the closed one silences the open one, like on a real kit.

## mute and solo pads

Each pad has mute and solo buttons, like a track.

- **Solo**: a click solos only this pad, [[key:Mod]]-click adds it to or removes it from the soloed pads.
- Pad **mute** is part of the project, like a track's mute.
- Pad **solo** is only for listening right now: it isn't saved with the project and isn't an undo step.

## where next

- [effects](/docs/making-music/effects): effects for each pad.
- [racks and macros](/docs/making-music/racks-and-macros): racks for layered and switchable sounds.
- [devices and presets](/docs/making-music/devices-and-presets#presets): save the sound of a pad's devices as presets.
