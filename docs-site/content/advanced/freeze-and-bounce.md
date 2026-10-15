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

- **bounce to new track**: renders the selected clips, with the track's effects and their tail (reverb and delay ring out, up to 30 seconds), to a new audio track right below. The original track is muted, so you hear the bounce.
- **bounce in place** (audio clips only): renders the clips without the track's effects and replaces them with the result.
- **consolidate**: joins the selected clips on each track into one clip.

With a time selection, its right-click menu also has **bounce to new track**: it bounces that range of every selected track, as one step. Tracks with nothing playing in the range are skipped.

Bouncing and consolidating are one undo step each.

## where next

- [tracks](/docs/making-music/tracks): groups and track menus.
- [audio clips](/docs/making-music/audio-clips)
- [exporting audio](/docs/import-export/exporting-audio): render the whole song to a file.
