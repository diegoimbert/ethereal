---
title: touch gestures
order: 20
summary: Scroll, zoom, select, move, trim and nudge clips with your fingers in the mobile beta.
platform: mobile
audience: everyone
covers:
  - mobile/pan-pinch-and-momentum #scrolling-and-zooming
  - mobile/tap-double-tap-long-press #tap-double-tap-and-long-press
  - mobile/panning-stops-following-the-playhead #following-the-playhead
  - mobile/empty-arrangement-double-tap-to-make-a-clip #making-a-clip
  - mobile/track-headers-on-touch #track-headers
  - mobile/the-track-sheet #the-track-sheet
  - mobile/sheets-close-with-back #sheets
  - mobile/moving-clips-select-first-or-long-press #moving-clips
  - mobile/trim-handles-only-on-selected-wide-enough-clips #trimming-clips
  - mobile/the-clip-action-bar #the-clip-action-bar
  - mobile/select-more-multi-select-without-shift #selecting-several-clips
  - mobile/nudge-by-one-grid-step #nudging-by-one-grid-step
  - mobile/time-range-with-handles #selecting-a-time-range
  - mobile/clip-sheet-typed-position #typing-a-position
---

A phone has no mouse, no right-click and no Shift key, so the [mobile beta](/docs/mobile/mobile-beta) maps them to gestures. This page covers the arrangement: scrolling and zooming, tracks, and clips. Editing notes, automation and the mixer is on [editing on touch](/docs/mobile/editing-on-touch).

## tap, double-tap and long-press

Three gestures stand for the mouse:

| gesture | what it does | on a computer |
|---|---|---|
| tap | selects, presses a button | click |
| double-tap (two taps within about 0.3 s) | opens or creates | double-click |
| long-press (hold still for about half a second) | opens the item's menu | right-click |

On Android, a long-press gives a short vibration when it registers. If your finger moves more than a few pixels, the touch becomes a drag instead.

## scrolling and zooming

- **Drag with one finger** to scroll. Sideways moves through time, up and down moves through the tracks. A drag that starts mostly in one direction stays locked to it, so a sideways swipe doesn't drift up or down.
- **Flick** and the view keeps gliding after you lift your finger. Touch the screen to stop it.
- **Pinch with your fingers side by side** to zoom in and out of time, around the point between your fingers.
- **Pinch with one finger above the other** to make the tracks taller or shorter.
- **Drag on the ruler** at the top to move the playhead (scrubbing).

## following the playhead

In the mobile beta the view follows the playhead by default, so the music stays on screen while it plays. As soon as you scroll or pinch while it's playing, following stops, so you can look elsewhere. Tap the **follow playhead** button (in the status row above the transport) to follow it again.

## making a clip

A new project's arrangement is empty and says **double-tap a MIDI track's lane to make a clip**. Double-tap in a MIDI track's lane to create an empty MIDI clip there, then double-tap the clip to [edit its notes](/docs/mobile/editing-on-touch#the-piano-roll-on-touch). There's no sample browser on mobile, so audio clips come from projects made on a computer.

## track headers

- **Tap** a header to select the track.
- **Tap the selected header again** to open its [track sheet](#the-track-sheet).
- **Long-press** a header for the track menu.
- **Long-press, then drag** a header to move the track up or down, or into a group.

On a phone, a header shows the track's name and its **mute**, **solo** and **arm** buttons, always visible and touch-sized. Tap them directly. A tablet also shows the volume fader and pan knob.

## the track sheet

The track sheet has everything a track header does on a computer, at touch size:

- **mute**, **solo** and **arm**, and **automation**, which shows the track's [automation lanes](/docs/mobile/editing-on-touch#automation-on-touch).
- A larger **volume** fader and **pan** knob.
- **name**: tap to rename the track.
- **move up** and **move down**, and the track menu's actions: cut, copy, paste, duplicate and delete.
- **devices**: the track's devices with on or off. You can't edit them on mobile yet ("edit devices on a computer for now").

## sheets

Sheets (the track sheet, a clip's position) slide up from the bottom of the screen:

- Drag the handle at the top up to make the sheet taller, or down to make it smaller or close it.
- Tap outside the sheet to close it.
- The system back button or back gesture closes the sheet instead of leaving the project. It also closes the clip editor.

In landscape, a sheet fills the screen.

## moving clips

A plain drag scrolls the view, even over a clip, so you can scroll a busy arrangement without moving anything by accident. To move a clip:

- **Tap it first** to select it, then drag it. Dragging a selected clip moves the whole selection.
- Or **long-press it, then drag** without lifting your finger. This works on a clip that isn't selected.

While you drag, a badge above your finger shows where the clip will land (snapped to the grid). Drag near an edge of the screen to scroll in that direction. Drag below the last track to put the clip on a new track. To cancel a drag, put a second finger down.

## trimming clips

Only a selected clip has trim handles: a wide touch target on each edge, half inside and half outside the clip. Drag one to shorten or lengthen the clip from that side.

A clip that is very narrow on screen (under about 24 pixels) has no handles. Zoom in, or type its start and length in the [clip sheet](#typing-a-position).

## the clip action bar

When clips are selected, a bar appears at the bottom of the arrangement with what you'd do with the keyboard or a right-click on a computer:

- **select more**: select several clips (below).
- **edit**: open the clip in the editor.
- **split**, **duplicate**, **copy**, **paste** and **delete**.
- **earlier** and **later**: [nudge](#nudging-by-one-grid-step) by one grid step.
- **position**: type the clip's start and length (one clip only).
- **more**: the clip's full menu, as a right-click on a computer shows it.

## selecting several clips

There's no Shift or Ctrl on a phone, so tap **select more** in the clip bar. Until you're done:

- A tap on a clip adds it to the selection or removes it.
- A drag on empty lane space draws a selection box.

The button then shows **done** and the number of selected clips: tap it to go back to normal taps. Select more also ends by itself when no clip is selected any more.

## nudging by one grid step

**earlier** and **later** move the selected clips by one grid step at the current zoom (one beat when the grid is off). Use them when a drag is too coarse for your finger. Clips never move before the start of the song.

## selecting a time range

On a computer you Shift-drag to select a stretch of time. On a phone, **long-press empty lane space, then drag**. The range gets two grips, like selecting text: drag either end to change it (it snaps to the grid). Dragging one grip past the other swaps them.

A range has its own bar: **copy**, **cut**, **paste**, **duplicate**, **loop it** (sets the loop to the range and turns looping on), **delete time** and **more**. What these do is explained in [arranging](/docs/making-music/arranging).

## typing a position

**position** in the clip bar opens a sheet where you type numbers instead of dragging:

- **start**: where the clip starts, as bar.beat.sixteenth, counted from 1. `3` is the start of bar 3, `3.2` is its second beat.
- **length**: how long it is, as bars.beats.sixteenths, counted from 0. `1` is one bar, `0.2` is two beats.
- **start edge** and **end edge**: **earlier** and **later** move that edge by one grid step.

Press Enter or tap elsewhere to apply a typed value. A value that doesn't make sense is put back as it was. Each change is one step you can undo.

## where next

- [Editing on touch](/docs/mobile/editing-on-touch): notes, automation and the mixer.
- [Clips](/docs/making-music/clips): what clips are and how they behave.
- [Undo and history](/docs/basics/undo-and-history).
