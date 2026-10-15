---
title: midi learn
order: 40
summary: Map the knobs, faders, pads and keys of a MIDI controller to controls in Ethereal.
platform: desktop
audience: advanced
covers:
  - audio-midi-setup/midi-controllers-just-work-no-device-list #your-controller-just-works
  - audio-midi-setup/midi-mapping-turn-on-midi-mode-click-a-control-move-a-knob #mapping-a-control
  - audio-midi-setup/right-click-any-control-learn-midi-remove-midi-mapping #from-a-controls-right-click-menu
  - audio-midi-setup/mappings-are-saved-in-the-project-not-globally #mappings-belong-to-the-project
  - audio-midi-setup/one-mapping-per-knob-learning-again-replaces-it #one-knob-one-target
  - audio-midi-setup/mapping-options-mode-min-and-max #mode-minimum-and-maximum
  - audio-midi-setup/mapped-keys-still-play-notes #mapped-keys-still-play-notes
  - audio-midi-setup/midi-learn-needs-the-desktop-app #desktop-only
---

MIDI learn connects a control on your MIDI controller (a knob, fader, pad or key) to a control in Ethereal: a device parameter, a track's volume, play/stop, and so on. You map it once, and moving the hardware moves the control on screen.

## your controller just works

There is no list of MIDI devices to set up. Every MIDI input connected to your computer opens by itself. Each MIDI track chooses which ports it listens to in its **in ▾** panel (all ports by default): see [recording](/docs/making-music/recording).

## mapping a control

1. Open the **midi mapping** tab in the left rail.
2. Turn on **midi mode**. Every control you can map lights up, and mapped ones are marked.
3. Click a highlighted control. The panel says it's learning (click the control again, or **cancel**, to stop).
4. Move a knob or press a key on your controller. The mapping appears in the panel's list.

[[key:Escape]] cancels a learn in progress. Turn **midi mode** off to use the app normally again: your mappings keep working.

### from a control's right-click menu

While MIDI mode is on, right-click a mappable control for **learn midi** and **remove midi mapping**.

### targets without a control

Some targets have no control on screen to click. Pick them from **learn other target…** in the panel:

- transport actions: play/stop, play, stop, record, loop, metronome, tap tempo, previous marker and next marker;
- arm, mute or solo of the selected track.

## mappings belong to the project

Mappings are saved in the project file, not in the app. They don't carry over to your other projects, and there is no global controller setup. Adding, changing and removing a mapping are normal edits that you can undo.

## one knob, one target

Each control on your controller (its port, channel and CC number or note) can drive only one target. If you learn a knob that is already mapped, it moves to the new target.

## mode, minimum and maximum

Each mapping in the list has:

- a **mode**:
  - **absolute**: the knob's position sets the value (most knobs and faders).
  - **relative (2's comp.)**, **relative (offset 64)**, **relative (sign bit)**: for endless encoders, which send "turn up" or "turn down" steps. Each controller uses one of these three encodings: check its manual, or try them.
  - **toggle**: each press switches the target on or off (pads, buttons).
- a **minimum** and **maximum**, in percent of the target's range. Setting the minimum above the maximum inverts the control.

Right-click a mapping for **learn again** or **remove mapping**, or click its **✕**.

## mapped keys still play notes

MIDI that a mapping uses still reaches your tracks. If you map a pad to play/stop, pressing it also plays a note on any armed or monitored instrument track.

## desktop only [[desktop]]

The web version has no hardware MIDI input yet, so its **midi mode** switch is off and says **midi learn needs the desktop app**.

## where next

- [recording](/docs/making-music/recording): MIDI inputs and channel filters per track.
- [the computer midi keyboard](/docs/making-music/computer-midi-keyboard): play notes without a controller.
- [audio and midi setup](/docs/making-music/audio-and-midi-setup)
