---
title: tempo, time signature and scale
order: 30
summary: Set the tempo and the time signature, change them during the song, and pick the project's scale.
platform: all
audience: beginner
covers:
  - transport-tempo/tempo-field-type-drag-arrows-shift-for-fine #setting-the-tempo
  - transport-tempo/the-tempo-field-edits-the-tempo-at-the-playhead-not-the-whol #songs-with-tempo-changes
  - transport-tempo/tap-tempo #tap-tempo
  - transport-tempo/time-signature-field-is-per-position #the-time-signature
  - transport-tempo/tempo-and-time-signature-changes-on-the-ruler-marker-lane #changes-during-the-song
  - transport-tempo/tempo-automation-lane-on-the-master-track #the-tempo-lane
  - transport-tempo/a-time-signature-change-mid-bar-cuts-that-bar-short #a-change-in-the-middle-of-a-bar
  - transport-tempo/project-scale-is-a-visual-guide-only #the-project-scale
---

This page shows how to set your song's tempo and time signature, how to change them partway through, and what the project's scale does. The fields are in the middle of the top bar, next to the position display.

## setting the tempo

The **bpm** field shows the tempo in beats per minute. You can change it in three ways:

- **Type** a value and press [[key:Enter]].
- **Drag** up or down on the field: one bpm per step. Hold [[key:Shift]] for fine steps of 0.01 bpm. A whole drag is a single undo step.
- Press [[key:ArrowUp]] or [[key:ArrowDown]] while the field has focus: one bpm per press, or 0.1 with [[key:Shift]].

The tempo goes from 20 to 999 bpm.

### tap tempo

Click **tap** in rhythm, at least twice, and Ethereal sets the tempo from the average time between your taps. Stop for more than 2 seconds and the next tap starts a new count. All the taps of one count are a single undo step.

### songs with tempo changes

If your song has tempo changes (see [below](#changes-during-the-song)), the bpm field shows and edits the tempo **at the playhead**, not the song's first tempo. Move the playhead into the part you want to change before you type.

## the time signature

The time signature field next to the tempo works the same way: it shows and edits the signature **at the playhead**. Type a value such as `7/8` and press [[key:Enter]].

To change the meter partway through the song, add a time signature change on the ruler instead.

## changes during the song

Right-click the ruler (its numbers or the markers band above them: they share one menu, with **add marker here** too) where you want a change and choose **add tempo change here** or **add time signature change here**. The changes show as small chips along the bottom edge of the ruler: the tempo ones read like ♩120, the signature ones like 3/4.

- **Drag** a chip to move it. It snaps to the grid; hold [[key:Alt]] to move it freely.
- **Right-click** a tempo chip to switch between **ramp to next point** (the tempo glides to the next change) and **hold tempo (no ramp)** (it jumps), or to delete it.
- **Right-click** a signature chip to pick a common signature or to delete it.

The tempo and the signature at bar 1 are always there: you can change their values, but not move or delete them.

### a change in the middle of a bar

A time signature change can sit on any beat, not only on a bar line. If you put one inside a bar, that bar ends early and bar 1 of the new signature starts at the change.

## the tempo lane

For finer tempo work, open the **tempo** lane on the master track (it's the master track's first automation lane; see [automation](/docs/making-music/automation)). It draws the tempo as a curve, with a handle for each tempo point:

- **Double-click** the lane to add a point, and double-click a point to remove it.
- **Drag** a point to change its time and its bpm. Hold [[key:Shift]] for fine steps of 0.01 bpm, and [[key:Alt]] for precise placement: off the grid, a tenth as far as the mouse. The point at the start only changes its bpm. [[key:Escape]] during the drag puts the points back and leaves nothing in the undo history.
- **Right-click** a point to turn **ramp to next point** on or off, or to delete it.
- Select a point and press [[key:Delete]] or [[key:Backspace]] to remove it.

## the project scale

Each project has a **scale** (a root note and a type, such as C minor). You set it in the project menu (☰), under **project scale**. Every MIDI track's [piano roll](/docs/making-music/piano-roll) uses it by default to highlight the notes that are in the scale.

The scale is only a visual guide. It doesn't stop you from playing or drawing other notes, and it doesn't correct anything.

## where next

- [Playing and moving around](/docs/basics/playing-and-navigating)
- [The piano roll](/docs/making-music/piano-roll)
- [Automation](/docs/making-music/automation)
