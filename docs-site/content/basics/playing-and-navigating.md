---
title: playing and moving around
order: 20
summary: Play and stop your song, understand the difference between the playhead and the insert marker, loop a section, and zoom and scroll the arrangement.
platform: all
audience: beginner
covers:
  - transport-tempo/play-stop-buttons-and-what-stop-does-twice #the-play-and-stop-buttons
  - transport-tempo/space-returns-to-where-you-started-shift-space-stops-in-plac #space-and-shiftspace
  - transport-tempo/space-works-everywhere-except-text-fields #space-works-everywhere
  - transport-tempo/record-loop-and-metronome-have-no-default-shortcut #record-loop-and-metronome-keys
  - transport-tempo/ruler-click-to-locate-and-scrub #the-ruler-moves-the-playhead
  - arrangement/click-on-a-lane-places-the-insert-marker-not-the-playhead #the-insert-marker
  - transport-tempo/follow-playhead-is-off-by-default #follow-the-playhead
  - transport-tempo/loop-brace-move-resize-toggle-draw-a-new-one #looping-a-section
  - arrangement/zoom-and-scroll-with-wheel-trackpad-middle-button #zoom-and-scroll
  - arrangement/zoom-out-stops-past-the-song-end #how-far-you-can-zoom-out
  - transport-tempo/position-display-before-zero-count-in #the-position-display
  - transport-tempo/metronome-settings-and-count-in-click #the-metronome
  - transport-tempo/count-in-and-punch-in-out #count-in-and-punch
---

This page covers the transport (play, stop, loop, metronome) and how to move around the arrangement. It helps to have a project open with a clip or two in it. If you don't have one yet, start with [your first loop](/docs/welcome/first-beat).

## the play and stop buttons

The transport is in the middle of the top bar.

- **Play** (▶) starts playback. While playing it turns into a pause button: press it to stop where you are.
- **Stop** (■) stops playback. Press it again while stopped and the playhead goes back to your [insert marker](#the-insert-marker) or selection. A third press takes it to the start of the song.

## space and shift+space

- [[action:transport.play]] plays. Pressed again, it stops and jumps back to where you were: the insert marker or the start of the time selection, or else the place where playback started. This makes it easy to listen to the same passage again and again.
- [[action:transport.stopToSelection]] also plays, but pressed while playing it stops **in place**, leaving the playhead where it is.

Both can be changed in the keyboard shortcuts (see [keyboard shortcuts](/docs/reference/keyboard-shortcuts)).

### space works everywhere

Space always plays and stops, wherever the focus is: on a button, a checkbox, a menu, a slider, a knob, a dialog or a plugin window. It never "presses" the focused button. The only exception is a text field, where space types a space.

## record, loop and metronome keys

In the default **Ethereal** key preset, **record**, **loop** and **metronome** have no key: use their buttons in the top bar, or the [command palette](/docs/basics/command-palette-and-settings). The **Ableton-like** preset adds keys for two of them: record [[action:transport.record]] and loop [[action:transport.loop]]. You can give any of them a key of your own in **keyboard shortcuts…** (in the command palette, or **settings › input**).

## the playhead and the insert marker {#the-insert-marker}

Ethereal has two different cursors, like Ableton Live:

- The **playhead** is where playback is, or where it will start. It's the line that moves while the song plays.
- The **insert marker** is where your edits go. A plain click on an empty part of a track, or on a clip's body, places a blinking insert marker there, on that track. It snaps to the grid; hold [[key:Alt]] while clicking to place it freely.

Paste, split, insert and the other edits land at the insert marker (or in your time selection), **never at the playhead**.

When the song is stopped, placing the insert marker also moves the playhead there, so play starts where you clicked. While it's playing, a click never moves the playhead: it only takes effect at the next stop.

### the ruler moves the playhead

To move the playhead itself, use the **ruler** along the top of the arrangement. Press on it to put the playhead there, and drag to scrub. It snaps to the grid ([[key:Alt]]: free). Clicking the ruler doesn't move your insert marker.

## follow the playhead

By default the view stays still while the song plays, and the playhead can run off the screen. To make the view follow it, turn on the **follow** button (the crosshair) in the arrangement's corner toolbar. With follow on, the playhead moves across the view until it reaches about 40 % of the width, then the view scrolls smoothly so it stays there.

On a touch screen, panning the view turns follow off.

## looping a section {#looping-a-section}

The **loop brace** is the bar along the ruler that marks the loop region. When the loop is on, playback repeats that region.

- Turn the loop on or off with the **loop** button in the top bar, or double-click the brace.
- Drag the brace to move it, and drag its edges to resize it.
- [[key:Shift]]-drag on the ruler to draw a new loop region.

All of these snap to the grid; hold [[key:Alt]] to move freely. Each change to the loop is one undo step.

## zoom and scroll

With a mouse or trackpad, in the arrangement:

| to | do |
|---|---|
| scroll up and down through the tracks | the wheel |
| scroll sideways through time | [[key:Shift]] + wheel, or swipe sideways on a trackpad |
| zoom in and out in time | [[key:Mod]] + wheel, or pinch on a trackpad |
| make every track taller or shorter | [[key:Mod+Shift]] + wheel |
| pan in any direction | drag with the middle mouse button |

A pinch always zooms. On Windows the zoom direction is flipped by default, so pushing the wheel forward zooms in like on a Mac.

All of this can be changed in **settings › input**: the zoom and scroll sensitivity, which modifier zooms (or whether the plain wheel zooms), inverting each direction, what the middle button does and what the mouse's back and forward buttons do. The command palette has **mouse and wheel settings…** to get there quickly, and **zoom to fit** to show the whole song.

### how far you can zoom out

Zooming out stops a little past the end of your song (the last clip or marker), and never narrower than 32 bars of 4/4. So in an empty project you can't zoom out very far: there's nothing to see.

## the position display

Between the transport buttons and the tempo, the top bar shows where the playhead is, twice:

- in **bars.beats.sixteenths**, following the time signature (for example 5.3.1);
- in **minutes:seconds**.

During a recording count-in, the bars count down from before the start: one bar before 1.1.1 is -1.1.1 (there's no bar 0).

## the metronome

The **metronome** button in the top bar turns the click on and off. The small button next to the transport opens the **metronome settings**: the click's volume, whether the first beat of each bar is accented, and its sound (**classic**, **wood** or **beep**). Each change is one undo step, and the settings are saved with the project.

When you record with a count-in, the click plays during the count-in even if the metronome is off.

## count-in and punch

The recording controls in the top bar have a **count-in** of 0, 1, 2 or 4 bars before recording starts, and a **punch in/out** button that records only inside the loop region. They're explained in [recording](/docs/making-music/recording).

## where next

- [Tempo, time signature and scale](/docs/basics/tempo-and-time-signature)
- [Projects and saving](/docs/basics/projects)
- [Keyboard shortcuts](/docs/reference/keyboard-shortcuts)
