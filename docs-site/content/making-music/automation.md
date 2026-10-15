---
title: automation
order: 110
summary: Draw how volume, pan and any device parameter change over the song, point by point.
platform: all
audience: beginner
covers:
  - automation/show-automation-lanes #showing-automation-lanes
  - automation/adding-points #adding-points
  - automation/editing-points #editing-points
  - automation/shift-snaps-values-to-round-numbers #round-values-with-shift
  - automation/curves #curves
  - automation/set-an-exact-value #set-an-exact-value
  - automation/time-ranges-in-a-lane #copying-and-moving-a-time-range
  - automation/lane-height-and-value-zoom #lane-height-and-value-zoom
  - automation/moving-an-automated-knob-doesn-t-stick #automated-knobs-follow-their-lane
  - automation/hide-vs-disable-vs-delete-a-lane #hide-disable-or-delete-a-lane
  - automation/device-on-off-automation #turning-a-device-on-and-off
  - automation/clip-envelopes #clip-envelopes
  - automation/automation-vs-modulation #automation-or-modulation
---

Automation makes a parameter change over time: a filter that opens during the build, a fade-out at the end, a reverb that grows on the last chorus. Each automated parameter has a **lane** under its track, with points you place on the timeline. Playback follows the line between the points. You need a track with something to automate: its volume and pan are always there, and every device parameter can be automated too.

## showing automation lanes

- Click the **automation** button in a track's header to open its lanes under the track. The first time, it shows the lanes that already have automation, or the volume lane if none do. The master track shows its tempo lane first.
- **+ parameter…** adds another lane: pick any parameter of the track (volume, pan, sends, each device's on/off and parameters). Turn on the **active** filter in that picker to list only parameters that already have automation. That choice is remembered on this computer.
- From a device: right-click any knob and choose **show automation lane**.

The **automation** tab of the editor drawer ([[action:view.editor]]) shows the selected track's lanes with their own ruler, if you prefer a bigger view.

## adding points

**Double-click** an empty spot in a lane to add a point. The point appears on the second press, so you can keep the button down and drag to place it exactly before letting go. A lane is created by its first point.

There is no pencil or freehand drawing: you build the line from points.

## editing points

- **Select**: click a point. [[key:Shift]]-click adds points to the selection, [[key:Mod]]-click toggles one. [[key:Mod]]-drag on empty space draws a selection box.
- **Move**: drag the selected points. Time snaps to the grid; hold [[key:Alt]] for precise placement, and hold [[key:Mod]] to lock the move to one direction (time or value). A tooltip shows the value and these keys while you drag.
- **Delete**: double-click a point, or select points and press [[action:edit.delete]].
- **Select all** the points of the focused lane: [[action:edit.selectAll]].
- **Nudge** with the arrow keys: up and down change the value (one step, or 1 %), left and right move by one grid step. With [[key:Shift]], up and down jump to the next whole value (such as 1 dB) and left and right move by a quarter step. [[action:automation.preciseUp]] and [[action:automation.preciseDown]] change the value by a tenth of the normal step.
- **Copy, cut, paste, duplicate** points with [[action:edit.copy]], [[action:edit.cut]], [[action:edit.paste]] and [[action:edit.duplicate]].
- Right-click a point for **cut**, **copy**, **paste**, **duplicate**, **set value…**, the curve type and **delete**.

### round values with shift

While you drag a point, hold [[key:Shift]] to move the value in whole steps of what it shows: 1 dB, 1 %, 1 semitone, 0.01 of pan, and so on. The point lands on a clean value. Parameters that only have steps (semitones, choices, on/off) always snap to them.

When a pixel is too coarse for the value you need, hold [[key:Alt]] while you drag: the point moves a tenth as far as the mouse, and off the grid. You can press or release it in the middle of a drag; the point carries on from where it is, without jumping. With [[key:Shift]] and [[key:Alt]] together, the value still moves in whole steps and only the time moves slowly.

### curves

- [[key:Alt]]-drag a segment (the line between two points) to bend it into a curve.
- The **curve** menu in the lane header sets **linear**, **step** or **smooth** for the segments after the selected points. Right-clicking a point offers the same choices.

### set an exact value

Right-click a point and choose **set value…**. Type the value in the parameter's unit (dB, Hz, %…), then press [[key:Enter]] to apply it or [[key:Escape]] to cancel.

## copying and moving a time range

Drag across empty space in a lane to select a time range in that lane. Then:

- [[action:edit.copy]], [[action:edit.cut]] and [[action:edit.delete]] copy, cut or delete the automation inside the range. The curve outside the range stays as it was.
- [[action:edit.paste]] pastes after the range, and [[action:edit.duplicate]] repeats the range right after itself.

You can paste into another parameter's lane: the values are converted to the new parameter's range.

## lane height and value zoom

- Drag a lane's bottom edge to make it taller or shorter. It moves in steps; hold [[key:Alt]] to resize freely. Double-click the edge to reset it.
- The **value scale** next to the lane header: drag it or scroll to move the visible range of values, [[key:Mod]]-scroll to zoom into the values, and double-click to reset.
- Stepped parameters open zoomed in, so each step is visible (a transpose lane shows ±3 semitones at first). Resize the lane or zoom the scale to see more.

## hide, disable or delete a lane

The lane header has three different buttons:

| button | what it does | the points |
|---|---|---|
| **×** | hides the lane | kept, and still played |
| **⏻** | disables the automation: playback ignores it | kept |
| **⌫** | deletes the lane | deleted (undo brings them back) |

## automated knobs follow their lane

Once a parameter has an enabled lane, its knob follows the lane, both during playback and when stopped. If you turn the knob, the change only lasts until the engine sends the automated value again: it doesn't stick. There is no automation recording and no "override" button.

To set the parameter by hand again, disable the lane with its **⏻** button, or delete the lane.

## turning a device on and off

A device's on/off switch can be automated, like any parameter: in the **+ parameter…** list it is **device on**, in the group of that device's parameters. While it is automated, the device's **power** button shows whether the automation has it on or bypassed. Clicking the button only changes the device where the lane doesn't decide.

## clip envelopes

A clip can carry its own automation, called **clip envelopes**. They belong to the clip: they move with it, repeat when the clip loops, and are copied with it.

- In the piano roll, the lane picker under the notes lists every parameter of the track. Parameters that already have an envelope in this clip come first.
- For audio clips, the warp editor has an envelope strip under the waveform.

While a clip plays, its envelope **replaces** the track's lane for that parameter; outside the clip, the track's lane applies. Turn an envelope **off** to keep it but let the track's lane play.

Audio clips also have two envelopes of their own, **clip gain** and **transposition**. These add to the clip's gain and transpose settings instead of replacing a track lane.

## automation or modulation

- **Automation** writes exact values on the timeline, one lane per parameter. Use it for changes that belong to a moment of the song.
- **Modulation** (LFOs, envelopes, macros) moves a parameter around its current value, continuously, by an amount you set. Use it for movement that repeats or follows the notes. See [modulation and sidechain](/docs/making-music/modulation-and-sidechain).

Both can drive the same parameter: the modulation moves around the automated value.

## where next

- [modulation and sidechain](/docs/making-music/modulation-and-sidechain): movement that follows an LFO, the notes or another track.
- [mixing](/docs/making-music/mixing): volume, pan and sends, which you can automate too.
- [devices and presets](/docs/making-music/devices-and-presets#the-parameter-menu): the parameter menu.
