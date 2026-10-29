---
title: arranging: selections and time edits
order: 100
summary: Use the grid, select clips or a span of time, cut, copy, duplicate and insert whole sections of the song, and mark its parts.
platform: all
audience: everyone
covers:
  - arrangement/grid-and-snap-alt-bypasses-everywhere #the-grid-and-snapping
  - arrangement/plain-drag-selects-clips-shift-drag-selects-time #selecting-clips-or-time
  - arrangement/plain-edit-keys-act-on-the-time-selection-when-one-exists #edit-keys-and-the-time-selection
  - arrangement/time-edits-insert-silence-delete-time-duplicate-paste #time-edits
  - arrangement/selecting-all-tracks-moves-markers-and-tempo-too #all-tracks-or-some
  - arrangement/paste-overwrites-time-clipboard-is-not-saved #pasting-time
  - arrangement/undo-restores-selections #undo-brings-your-selection-back
  - arrangement/markers-locators #markers
  - arrangement/arrangement-shortcuts-need-the-arrangement-focused #shortcuts-need-the-arrangement
---

Once a song has a few parts, you'll want to move whole sections around: make the chorus longer, remove a bar everywhere, or repeat the intro. This page covers the grid, the two kinds of selection, the time edits and markers. Read [clips](/docs/making-music/clips) first.

## the grid and snapping

The **grid** picker in the corner above the track headers sets the grid of the arrangement:

- **adaptive (narrow)**, **adaptive** and **adaptive (wide)** follow the zoom: the closer you zoom in, the finer the grid.
- **1 bar**, **1/4**, **1/8** and **1/16** are fixed steps.
- **off** turns snapping off.

Every drag in the arrangement snaps to the grid: moving and trimming clips, slip edits, the loop brace, markers, tempo points, time selections and take swipes. Hold [[key:Alt]] while dragging or clicking to ignore the grid for that gesture.

## selecting clips or time

The arrangement has two kinds of selection:

- **Clips**: drag over empty lane space to draw a box. The clips it touches are selected.
- **Time**: hold [[key:Shift]] and drag. This selects a span of time across the tracks you drag over, whatever clips are there. It cuts through clips.

You can press or release [[key:Shift]] during the drag to switch between the two. A click clears the time selection and places the insert marker instead (see [playing and moving around](/docs/basics/playing-and-navigating)).

## edit keys and the time selection

When a time selection exists, the plain edit keys act on that section instead of on whole clips: [[action:edit.copy]], [[action:edit.cut]], [[action:edit.paste]], [[action:edit.duplicate]] and [[action:edit.delete]]. Clips are cut at the edges of the selection, and only that span is copied or removed.

Without a time selection, the same keys act on the selected clips (or tracks). [[action:edit.paste]] pastes whatever you copied last: clips, tracks or time.

## time edits

These act on the time selection. Right-click inside it for the same list as a menu.

| edit                  | shortcut                                               | what it does                                                                                                                            |
| --------------------- | ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- |
| cut time              | [[action:edit.cut]] or [[action:time.cut]]             | Copies the section, then removes it. Later material moves left.                                                                         |
| copy time             | [[action:edit.copy]] or [[action:time.copy]]           | Copies the section.                                                                                                                     |
| paste after selection | [[action:edit.paste]]                                  | Pastes the copied section right after the selection, over what is there.                                                                |
| paste time            | [[action:time.paste]]                                  | Inserts the copied section at the start of the selection and pushes what follows to the right.                                          |
| split at selection    | [[action:edit.split]]                                  | Splits the clips at both edges of the selection.                                                                                        |
| mute selection        | [[action:track.toggleMute]]                            | Mutes the clips in the section. A clip that crosses an edge is split there, so only its part inside is muted. Press it again to unmute. |
| duplicate time        | [[action:edit.duplicate]] or [[action:time.duplicate]] | Puts a copy of the section right after it, over what is there.                                                                          |
| insert silence        | [[action:time.insertSilence]]                          | Inserts empty time as long as the selection at its start. Everything after it moves right.                                              |
| delete time           | [[action:edit.delete]] or [[action:time.delete]]       | Removes the section. Later material moves left.                                                                                         |

The menu also has **paste over selection** and the bounce commands (see [freeze, flatten and bounce](/docs/advanced/freeze-and-bounce)). Each time edit is one undo step.

### all tracks or some

If the selection covers every track, a time edit changes the whole song: markers, tempo changes and time signature changes move with it. If it covers only some tracks, only their clips, takes and automation move.

### pasting time

Paste and duplicate go over what is already there. To push the existing material to the right instead, use **paste time** ([[action:time.paste]]).

The time clipboard lives in the engine for this session only. It is lost when you restart the app.

## undo brings your selection back

[[action:edit.undo]] and [[action:edit.redo]] also bring back the selection you had at that step: the time selection, the piano roll's section and the selected clips and notes. So you can undo an edit and redo it on the same span right away.

## markers

Markers name the parts of a song (intro, verse, chorus) and let you jump to them. They live in the **markers** lane above the tracks.

- Double-click empty space in the lane to add a marker there, or click **+** in the lane's corner. **+** adds it at the insert marker, or at the playhead while playing, so you can mark what you hear.
- Click a marker to jump there.
- Drag a marker to move it. It snaps to the grid ([[key:Alt]]: free).
- Double-click a marker to rename it.
- Right-click a marker for **jump to marker**, **rename** and **delete marker**.

## shortcuts need the arrangement

Clip and time shortcuts (split, duplicate, delete, the arrow keys for takes) work when the arrangement has the keyboard focus. Clicking anywhere in it gives it the focus. [[action:edit.deselect]] clears the selection and closes the piano roll from anywhere.

## where next

- [automation](/docs/making-music/automation): change settings over time.
- [the keyboard shortcuts](/docs/reference/keyboard-shortcuts): every shortcut in one place.
- [undo and history](/docs/basics/undo-and-history): go back to any step.
