---
title: audio clips: fades, warp and reverse
order: 90
summary: Shape audio clips with fades and crossfades, set their gain, play them backwards, and make them follow the tempo with warp.
platform: all
audience: everyone
covers:
  - clips/fade-handles-on-audio-clips #fades
  - clips/crossfades-between-touching-clips #crossfades
  - clips/clip-gain-changes-the-waveform-height #clip-gain
  - clips/reverse #reverse
  - clips/warp-on-off-repitch-or-complex #warp-on-or-off
  - clips/warp-markers #warp-markers
---

Audio clips have a few edits of their own on top of the ones every clip has (see [clips](/docs/making-music/clips)). Select an audio clip to follow along. Most of these settings are also in the inspector.

## fades

Hover over an audio clip, or select it, to see its fade handles at the top corners of the clip's body.

- Drag a handle sideways to set the length of the fade-in or fade-out.
- Drag the small handle in the middle of a fade up or down to bend its curve. Double-click it to make the fade straight (linear) again.
- Right-click the clip and choose **equal-power fades** or **linear fades** to switch the curves of every selected clip.

The inspector's **fade in** and **fade out** fields take exact lengths.

## crossfades

A crossfade blends the end of one clip into the start of the next one on the same track.

- Drag a fade handle past the clip's edge into a clip that touches or overlaps it. This creates a crossfade, centred on the boundary.
- Drag the handle of an existing crossfade to resize it. Drag it back to almost nothing to remove it.
- Or right-click a clip: **crossfade with previous clip** and **crossfade with next clip** make a crossfade half a beat long (shorter for short clips). **remove crossfade with previous clip** / **with next clip** take it away.

## clip gain

The inspector's **gain** field turns a clip up or down. The waveform grows or shrinks with it, so what you see matches what you hear. Parts that would go past full scale are drawn flattened, with a mark in the clip's color, as a warning that they may distort.

## reverse

Press [[action:clip.reverse]] (or right-click and choose **reverse**) to play the selected clips backwards.

- An audio clip shows a **rev** badge in its title bar. Press again (**unreverse**) to play it forwards. A trimmed clip plays the same part of the file, backwards. Fades stay at the clip's edges.
- A MIDI clip has its notes and controllers mirrored in time. That edits the notes, so press [[action:edit.undo]] to get them back.

The inspector also has a **reverse** switch.

## warp

Warp makes an audio clip follow the project's tempo: change the tempo and the clip speeds up or slows down with it. Click an audio clip to open its editor at the bottom of the window, on the **warp** tab.

### warp on or off

- **mode** turns warping off or picks how the audio is stretched:
  - **none** turns warp off: the clip plays at its original speed whatever the tempo.
  - **repitch** changes the speed and the pitch together, like a record played faster or slower.
  - **complex** keeps the pitch while changing the speed. Use it for vocals and full mixes.
- With several audio clips selected, the mode you pick applies to all of them. The clip menu's **warp mode** does the same.
- The toolbar shows the file's tempo (**source 120.00 bpm**), or **source tempo unknown**. When you first pick a mode other than **none**, Ethereal guesses it from the file's length: the tempo between 80 and 160 bpm at which the file lasts 1, 2, 4, 8… bars. It doesn't listen to the audio, so the guess is right for loops cut to whole bars; for anything else, set the tempo with warp markers.
- **transpose** changes the clip's pitch in semitones.

If the engine you use has no time-stretching, the toolbar says **complex plays as repitch here**.

### warp markers

Warp markers pin a moment of the audio to a beat, to fix timing that drifts. With warp on:

- Double-click the waveform to add a marker.
- Drag a marker to move that moment of the audio to another beat. It snaps to sixteenth notes. Hold [[key:Alt]] to move it freely.
- Double-click a marker, or select it and press [[key:Delete]], to remove it.

A marker only stretches the audio between its neighbours. The first marker you add also pins the start and the end of the file, so the rest of the clip doesn't move.

## where next

- [clips](/docs/making-music/clips): move, trim, split and loop.
- [automation](/docs/making-music/automation): change volume and other settings over time.
- [takes and comping](/docs/making-music/takes-and-comping): build one performance from several recordings.
