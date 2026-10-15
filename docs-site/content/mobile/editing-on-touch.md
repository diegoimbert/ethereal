---
title: editing on touch
order: 30
summary: Edit notes in the piano roll, draw automation and mix with faders in the mobile beta.
platform: mobile
audience: everyone
covers:
  - mobile/clip-editor-and-piano-roll-on-touch #the-clip-editor
  - midi-editing/piano-roll-on-touch-select-and-draw-modes #the-piano-roll-on-touch
  - mobile/automation-on-touch #automation-on-touch
  - automation/automation-on-touch #automation-on-touch
  - mobile/the-mixer-view-and-fine-mode #the-mixer
---

This page covers editing inside a clip, automation and mixing in the [mobile beta](/docs/mobile/mobile-beta). It assumes you know the basic gestures from [touch gestures](/docs/mobile/gestures): tap, double-tap, long-press, and dragging a selected item.

## the clip editor

**Double-tap a clip** (or select it and tap **edit** in the clip bar) to open its editor. It opens in a panel over the arrangement, between the top bar and the transport, so you can play and stop while you edit. On a phone it takes the full height; on a tablet, the lower part of the screen.

- A MIDI clip opens in the piano roll, an audio clip in the audio clip editor. The tabs at the top switch between the editors the clip has, as on a computer.
- Close it with the **close editor** button (✕) or the system back gesture.

## the piano roll on touch

The piano roll has two modes, switched by the **draw** toggle in its toolbar. A finger can't both select and draw without a modifier key, so you choose.

**Select mode** (the default):

- **Tap a note** to select it. Tap empty space to deselect.
- **Double-tap empty space** to add a note. **Double-tap a note** to delete it.
- **Drag a selected note** to move it. Drag its edge to make it longer or shorter.
- **Drag empty space** to scroll.
- **Long-press, then drag** on empty space to draw a selection box around several notes. Long-press then drag on a note moves it even if it isn't selected.
- **Long-press a note** for its menu.

**Draw mode**:

- **Tap** to add a note.
- **Drag on empty space** to draw a note as long as your drag.
- **Drag a note** to move it.

In both modes:

- **Pinch with your fingers side by side** to zoom time, and **one above the other** to make the keys taller or shorter.
- **Tap a key** on the keyboard at the left to hear that note.

On a phone the velocity and expression lanes are hidden, so the notes get the room. Tap **velocity** in the editor's top bar to show them. Everything else works as described in [the piano roll](/docs/making-music/piano-roll).

## automation on touch

To see a track's automation, tap its header to select it, tap it again to open the [track sheet](/docs/mobile/gestures#the-track-sheet), then tap **automation**. The lanes appear under the track in the arrangement. Then, in a lane:

- **Double-tap** to add a point. **Double-tap a point** to delete it.
- **Long-press a point, then drag** to move it.
- **Long-press empty space in the lane, then drag** to select a range of time in the lane.
- A plain drag scrolls the view, so you don't move points by accident.

What automation does, and the rest of its options, are explained in [automation](/docs/making-music/automation).

## the mixer

The button at the right end of the transport switches between the tracks and the mixer. The mixer shows a channel strip per track, side by side (scroll sideways for more), with the master strip pinned on the right. Each strip has:

- the track's name and its pan knob,
- a tall volume fader with the track's level meter, and the level in dB under it,
- **mute**, **solo** and **arm**.

VCA tracks have no strip, since they don't carry audio of their own. Tap the same button again to go back to the tracks.

### fine control

Faders and knobs never jump to where you touch them: they move from their current value as you drag. For small changes, slide your finger sideways while you drag up or down. The further you move off to the side, the slower the control follows: about half speed from a short distance away, then a quarter, then a tenth.

The same fine control works for the volume fader and pan knob in the track sheet and on a tablet's track headers. **Double-tap** a fader to reset it to 0 dB, or the pan knob to reset it to the center.

## where next

- [The piano roll](/docs/making-music/piano-roll): everything about notes, quantize and velocity.
- [Mixing](/docs/making-music/mixing): volume, pan, sends and groups.
- [Ethereal on your phone](/docs/mobile/mobile-beta): what the beta can and can't do.
