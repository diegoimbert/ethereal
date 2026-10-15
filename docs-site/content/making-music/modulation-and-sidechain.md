---
title: modulation and sidechain
order: 190
summary: Move parameters with LFOs, envelopes and other modulators, and let one track control a device on another.
platform: all
audience: advanced
covers:
  - racks-modulation/adding-a-modulator #adding-a-modulator
  - racks-modulation/mapping-a-modulator-to-parameters #mapping-a-modulator
  - racks-modulation/modulation-depth-chips-and-rings #depth-rings-and-chips
  - racks-modulation/modulator-types #modulator-types
  - racks-modulation/sidechain-a-device #sidechain
  - racks-modulation/sidechain-from-your-own-track-or-a-muted-track #keying-from-your-own-track-or-a-muted-track
---

**Modulation** moves a parameter by itself: a filter that wobbles with an LFO, a cutoff that opens with each note, a volume that follows another track. A **sidechain** lets a device listen to another track, the classic case being a compressor on the bass that ducks every time the kick hits. Read [devices and presets](/docs/making-music/devices-and-presets) first.

Modulation is different from [automation](/docs/making-music/automation#automation-or-modulation): automation writes values on the timeline, while modulation moves a parameter around the value you set, continuously.

## adding a modulator

Hover a device's header and click its **+** (add modulator) button, then pick a type (see below). The device's right-click menu has the same choices (**add LFO**, **add envelope**…). The modulator appears on the device's card, with its own controls.

Only devices directly on a track's chain can hold modulators: a device inside a rack chain or a drum pad can't. To modulate devices inside a rack, add the modulator to the rack itself.

## mapping a modulator

A modulator does nothing until you map it to parameters. Two ways:

- Click the modulator's **map** button: every parameter it can reach lights up. Click the parameters it should move. Hold [[key:Shift]] while clicking to keep mapping more; press [[key:Escape]] to stop.
- Drag the modulator's handle onto a parameter.

A modulator can reach the parameters of its own device. A modulator on a rack can reach every device inside the rack (and the rack's chain selector).

### depth rings and chips

Each mapping shows on its parameter:

- a **ring** on the knob, in the modulator's colour, from the knob's value to the value plus the depth. While the song plays, a dot shows where the modulation has moved the parameter right now;
- a **chip** under the control. Drag the chip up or down to set the depth, double-click it to set the depth to 0, right-click it to **invert** the modulation or **remove** it.

New mappings start at 50 % depth. The modulator's card also lists its targets with their depths.

## modulator types

| type | what it does | main controls |
|---|---|---|
| **LFO** | a repeating wave | shape (sine, triangle, saw up or down, square), rate or synced rate, phase, retrigger (**free**, on each **note**, or from the **transport**), fade in |
| **envelope** | a shape that starts with each note | attack, decay, sustain, release, velocity |
| **envelope follower** | follows the loudness of a sound | attack, release, gain; it listens to the device's input, or to another track you pick |
| **steps** | a sequence of 1 to 16 values | number of steps, rate, smooth, and a value per step |
| **random** | a new random value at each step | rate or synced rate, smooth |
| **keytrack** | follows the note you play | the low and high keys of its range |
| **velocity** | follows how hard you play | the low and high velocities of its range |

## sidechain

Some devices have a sidechain input: the **compressor**, **limiter**, **gate**, **multiband compressor** and **auto filter**, and plugins that accept a sidechain. Their header shows a **sidechain** menu.

1. In the sidechain menu, pick the source track. **no sidechain** turns it off.
2. A second menu then picks where to listen on that track: **pre FX** (before its devices), **post FX** (after its devices) or **post mixer** (after its volume and pan), on the track itself or on one of its rack chains or drum pads.

The device then reacts to the source instead of to its own input: a compressor on the bass, keyed from the kick, turns the bass down on every kick. Choices that would create a loop are greyed out, with the reason.

### keying from your own track or a muted track

- A device can listen to its own track's **pre FX** signal, or to a chain of a rack placed before it on the same track. Its own track appears in the list as **(this track)**.
- Muting or soloing the source track doesn't cut the sidechain: a muted track still keys the device. You can use a muted track as a pure trigger, for example a muted kick that only drives a ducking compressor.

## where next

- [racks and macros](/docs/making-music/racks-and-macros#macros): macros, the hand-controlled cousin of modulators.
- [effects](/docs/making-music/effects): the devices you will modulate most.
- [automation](/docs/making-music/automation): changes written on the timeline.
