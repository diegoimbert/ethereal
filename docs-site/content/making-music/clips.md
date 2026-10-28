---
title: clips
order: 20
summary: Create, select, move, copy, trim, split, loop and rename clips in the arrangement.
platform: all
audience: beginner
covers:
  - clips/grab-clips-by-their-title-bar #moving-a-clip
  - clips/cmd-ctrl-drag-copies-not-alt #copying-while-dragging
  - clips/clip-selection-modifiers #selecting-clips
  - clips/click-opens-the-clip-editor #opening-a-clip
  - clips/double-click-expands-the-editor-and-inspector #double-clicking-a-clip
  - clips/moving-onto-other-clips-trims-or-deletes-them #dropping-onto-other-clips
  - clips/moving-clips-between-tracks-and-to-new-tracks #moving-to-another-track
  - clips/trimming-limits-and-leading-gaps #trimming
  - clips/stretch-edge-cmd-ctrl-shift-drag #stretching
  - clips/slip-edit-cmd-ctrl-shift-drag #slip-editing
  - clips/create-midi-clips-with-double-click #making-a-midi-clip
  - clips/drag-and-drop-audio #adding-audio
  - clips/split-duplicate-loop-reverse-shortcuts #split-duplicate-loop-and-reverse
  - clips/paste-goes-to-the-insert-marker-and-tiles #copy-and-paste
  - clips/rename-clips #renaming-a-clip
  - clips/clip-and-track-colors #clip-colors
  - clips/editing-several-clips-in-the-inspector #editing-several-clips-at-once
---

Clips are the blocks of music on a track: MIDI clips hold notes, audio clips play a piece of an audio file. This page covers the edits every clip shares. For fades, warping and other audio-only edits, see [audio clips](/docs/making-music/audio-clips).

## making a midi clip

Double-click an empty spot in a MIDI track's lane to make a one-bar clip there. To choose its length, double-click and keep holding the second press, then drag: the clip covers the grid cells you drag over. Hold [[key:Alt]] to ignore the grid.

## adding audio

Drag audio files from your computer, or samples from the browser, onto an audio track's lane. The clips start where you drop. Several files go one after another. Drop below the last track to get a new audio track for each file. MIDI tracks don't accept audio. While a file imports, an **importing… %** placeholder shows its progress. Hold [[key:Alt]] while dropping to ignore the grid.

You can also use **import audio…** ([[action:file.import]]). See [importing audio](/docs/import-export/importing-audio).

## selecting clips

- Click a clip to select it (its track is selected too).
- [[key:Shift]]-click adds a clip to the selection.
- [[key:Mod]]-click adds or removes one clip.
- A plain click on a clip that is already selected keeps only that one.
- Drag over empty lane space to draw a box: every clip it touches is selected.

Clicking a clip also places the insert marker at its start. While playback is stopped, the playhead moves there too, so **play** starts from the clip.

## opening a clip

Clicking a MIDI clip opens it in the piano roll at the bottom of the window. Clicking an audio clip (without dragging and without modifier keys) opens its editor, with the warp settings and envelopes. Clicking elsewhere closes the editor again, unless you pinned it. Double-clicking a clip's body opens it too.

### double-clicking a clip

Double-clicking a clip (its title bar or its body, or a take's clip in a take lane) opens it in its editor, and also expands the editor drawer and the inspector if you had collapsed them to a bar. Each comes back at the size you last gave it. A panel that is already open keeps its size. You can collapse them again with their **×**: a plain click on a clip keeps them collapsed.

It works as a toggle: when the editor drawer and the inspector are both expanded, double-clicking the clip that is already open collapses both again. Each remembers its size for the next time, and the clip stays selected and open, so another double-click on it brings both back. Double-clicking a different clip instead shows that clip in the editor and keeps both panels open.

On a small window, an expanded panel is shrunk if needed so that the arrangement keeps its minimum size, the same limit as when you drag a panel's edge.

## moving a clip

Grab a clip by its title bar, the colored band at the top, and drag it. The rest of the clip (its body) lets clicks through to the lane, so dragging the body draws a selection box instead of moving the clip. This is what lets you place the insert marker or reach the fade handles inside a clip.

Moves snap to the grid. Hold [[key:Alt]] while dragging to move freely. [[key:Escape]] cancels a drag in progress. Every drag is one undo step.

### copying while dragging

Hold [[key:Mod]] when you release the drag to drop a copy and leave the original in place. You can press or release [[key:Mod]] during the drag to switch between moving and copying. [[key:Alt]] does not copy in Ethereal: it turns snapping off.

### moving to another track

Drag up or down to move clips to another track of the same kind: audio clips to audio tracks, MIDI clips to MIDI tracks. If any of the selected clips can't go where you point, they all stay on their tracks.

Drag below the last track to create new tracks for the clips. Ghost lanes show the tracks that will be made. The new tracks and the move are one undo step.

### dropping onto other clips

A clip that lands on other clips of the same track makes room for itself. This happens whether you drop, paste or resize it. Clips it covers completely are deleted. Clips it covers partly are trimmed. A clip it lands inside is split around it. Crossfades are kept.

## trimming

Drag a clip's left or right edge to make it shorter or longer. Trims snap to the grid. Hold [[key:Alt]] to trim freely.

- An audio clip that isn't looped can't get longer than the rest of its file. With loop on, it repeats as far as you drag.
- Dragging the start to the left past the beginning of the content leaves empty space (silence, or no notes) before it. Drag back right to remove the gap.
- The shortest clip is a sixteenth of a beat.

### stretching

Hold [[key:Mod+Shift]] while you drag an edge to stretch the clip instead: its content plays faster or slower so that it fills the new length. A MIDI clip's notes, expression and loop scale with it. An audio clip is warped to play its audio over the new length (an unwarped clip gets warped first), and its fades and envelopes scale too. In repitch mode the pitch follows the speed. With several clips selected, all of them stretch, in one undo step.

## slip editing

Slip editing moves what plays inside a clip without moving the clip itself. Hold [[key:Mod+Shift]] and drag the clip's body. It snaps to the grid ([[key:Alt]]: free), and [[key:Escape]] cancels. With several clips selected, all of them slip.

The content stops at the ends of the audio file (for MIDI, at the last note). So a clip that already shows the whole file has nothing to slip.

## split, duplicate, loop and reverse

These shortcuts act on the selected clips while the arrangement has the focus:

| action                                                           | shortcut                    |
| ---------------------------------------------------------------- | --------------------------- |
| split at the insert marker (or at the edges of a time selection) | [[action:edit.split]]       |
| duplicate right after the clip                                   | [[action:edit.duplicate]]   |
| loop on / off                                                    | [[action:clip.toggleLoop]]  |
| reverse                                                          | [[action:clip.reverse]]     |
| mute / unmute (or the clips in the time selection)               | [[action:track.toggleMute]] |
| consolidate into one clip per track (or the time selection)      | [[action:clip.consolidate]] |
| bounce to a new track (or the time selection)                    | [[action:clip.bounce]]      |
| delete                                                           | [[action:edit.delete]]      |

The clip's right-click menu has the same actions, plus **mute**, **rename** and, for MIDI clips, **open in piano roll**. Reverse is explained in [audio clips](/docs/making-music/audio-clips#reverse).

A looped clip shows a loop icon in its title bar. Extend it to the right to repeat its content.

## copy and paste

[[action:edit.copy]], [[action:edit.cut]] and [[action:edit.paste]] work on the selected clips. Pasted clips start at the insert marker (or where you right-clicked) and keep their spacing. The marker then moves to their end, so pasting again continues the pattern.

Clips are pasted back onto their own tracks. If they all came from one track and you select another track of the same kind, they go there instead.

If you made a time selection, these keys act on that section instead of whole clips. See [arranging](/docs/making-music/arranging#edit-keys-and-the-time-selection).

## renaming a clip

Right-click a clip and choose **rename**. Type in its title bar, then press [[key:Enter]] or click away. [[key:Escape]] cancels. If you clear the name, the clip gets its default name back (an audio clip shows its sample's name). A new MIDI clip has no name of its own: it shows its track's name, and follows the track when you rename it. **rename** is only offered when one clip is selected. The inspector also has a name field for the selected clip.

## clip colors

Select one or more clips and pick a color in the inspector, or right-click them and choose **change color…** to pick it right there (every selected clip changes, as one undo step). The first swatch, **use the track color**, makes the clips follow their track's color again. To change a track's color, see [tracks](/docs/making-music/tracks#colors).

## editing several clips at once

Select several clips to edit them together in the inspector: mute them, give them a color, and for audio clips set gain, transpose, reverse and warp.

- Typing a value sets it on every clip.
- Dragging a field or stepping it with the arrow keys moves every clip by the same amount, so their differences are kept.
- A field whose clips have different values shows them as mixed.

## where next

- [audio clips](/docs/making-music/audio-clips): fades, crossfades, gain, reverse and warp.
- [the piano roll](/docs/making-music/piano-roll): edit the notes inside a MIDI clip.
- [arranging](/docs/making-music/arranging): edit sections across tracks.
