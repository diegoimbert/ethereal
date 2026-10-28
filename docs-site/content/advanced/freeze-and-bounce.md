---
title: freeze, flatten and bounce
order: 70
summary: Render tracks and clips to audio to save cpu, commit to a sound, or join clips together.
platform: all
audience: advanced
covers:
  - clips/freeze-locks-clips-and-devices #freezing-a-track
  - clips/freezing-a-group-locks-its-tracks #freezing-a-group
  - clips/flatten-is-permanent #flatten
  - clips/bounce-to-new-track-bounce-in-place-consolidate #bounce-and-consolidate
---

Instruments and effects use cpu while they play. When a track is finished, or your computer struggles, you can render it to audio: Ethereal then plays the recording instead of running its devices. Freezing can be undone at any time, flattening makes it permanent, and bouncing renders a part to a new clip.

## freezing a track

Right-click a track's header and choose **freeze track** (with several tracks selected, it freezes them all). Ethereal renders the track and then plays that render.

While a track is frozen:

- a snowflake shows in its header;
- its clips, devices and device automation are locked: you can't edit them;
- its volume, pan, mute, solo and sends still work.

To edit it again, click the snowflake (or right-click the header and choose **unfreeze track**). The track plays its clips and devices again.

Renders run one at a time, in the background. The track's header shows the progress: click it to cancel, or choose **cancel freeze** in the track's menu.

## freezing a group

A group track can be frozen too: it then plays one render of everything inside it, with the group's own devices. While the group is frozen, the tracks inside show a lock. You can't edit, move, solo or mute them until you unfreeze the group, and their sends to return tracks stop.

A group can't be frozen when one of its tracks sends its output outside the group.

## flatten

**flatten track** (in the menu of a frozen track) makes the freeze permanent: the track's clips and devices are replaced by one audio clip of the render. It asks first, and names the tracks it will flatten.

- A frozen MIDI track becomes an audio track.
- A frozen group becomes one audio track, and the tracks inside it are removed.

> [!warning]
> Flatten throws away the clips and devices. Only undo ([[action:edit.undo]]) brings them back: once you can't undo any more, they're gone. Use freeze if you might want to change the track later.

## bounce and consolidate

Select clips, right-click one of them, and choose:

- **bounce to new track** ([[action:clip.bounce]]): renders the selected clips, with the track's effects and their tail (reverb and delay ring out, up to 30 seconds), to a new audio track right below. The original track is muted, so you hear the bounce.
- **bounce in place** (audio clips only): renders the clips without the track's effects and replaces them with the result.
- **consolidate** ([[action:clip.consolidate]]): joins the selected clips on each track into one clip.

With a time selection, the same shortcuts and its right-click menu act on the range instead:

- **bounce to new track** bounces that range of every selected track, as one step.
- **consolidate** joins that range of every selected track into one clip per track, audio or MIDI. The parts of the clips outside the selection stay as they are.

Tracks with nothing playing in the range are skipped.

Bouncing and consolidating are one undo step each.

## converting an instrument to a sampler

You can play a finished instrument without running it: right-click it on a track's device chain (an instrument rack, a synth), or inside an instrument rack, and choose **convert to sampler…**. Ethereal records the instrument note by note, then replaces it in its place with a multisampler that plays those recordings. The recordings are the instrument alone: the MIDI effects before it, the rack's other chains and its chain's volume stay as they are around the sampler, and the rack macros that move its settings are recorded as they are set. The recording uses far less cpu, but it is a snapshot: edits to the original sound don't reach it, and a held note ends at the end of its sample.

The dialog chooses what to record:

- **notes**: the keys to record. Pick a preset (every note, every few semitones, every octave), or click keys on the keyboard to add or remove them; drag across keys to add or remove several at once, or right-drag to remove them. A preset spans your lowest to highest picked key (C1 to C6 to start with), and editing the keys switches the preset to **custom**. Notes between recordings play the nearest one, pitched; notes outside them use the nearest edge recording. Every note gives the best quality; every third or fourth note is much faster.
- **velocities**: how hard each note is played when recorded. A preset spreads them evenly up to 127; **configure…** lists them so you can change, add or remove one (up to 16), which switches the preset to **custom**. Each velocity you play uses the nearest recorded one. One velocity is fastest and keeps playing soft quieter; more keep the instrument's response to playing soft or hard.
- **offset**: how long each recording runs before its note (0.2 s to start with), so the instrument is already running when the note plays. The multisampler's **offset** knob skips that much of every sample; lower it a little (to 0.19 s, say) to keep what the instrument plays just before the note.
- **note length** and **release tail**: how long each note is held and how much of its decay is kept.

The dialog shows how many samples the settings produce. Conversion runs like a freeze, one render at a time, and shows its progress in the track header. The result is one undo step.

## where next

- [tracks](/docs/making-music/tracks): groups and track menus.
- [audio clips](/docs/making-music/audio-clips)
- [exporting audio](/docs/import-export/exporting-audio): render the whole song to a file.
