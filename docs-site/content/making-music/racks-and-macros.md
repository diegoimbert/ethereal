---
title: racks and macros
order: 180
summary: Combine devices into racks with parallel chains, switch between sounds, and control many parameters from 8 macro knobs.
platform: all
audience: advanced
covers:
  - racks-modulation/group-a-device-into-a-rack #making-a-rack
  - racks-modulation/rack-chains #chains
  - racks-modulation/chain-zones-keys-velocity-chain-select #zones-keys-velocity-and-chain-select
  - racks-modulation/rack-content-rules-and-nesting-depth #what-a-rack-can-hold
  - racks-modulation/rack-macros #macros
  - racks-modulation/rack-presets-keep-the-whole-rack #rack-presets
---

A rack is a device that holds other devices, in one or more **chains** that play side by side. With a rack you can layer two synths, split the keyboard between a bass and a lead, run an effect in parallel with the dry sound, or switch sounds with one knob. Each rack also has **8 macro knobs** that can each control many parameters inside it. Read [devices and presets](/docs/making-music/devices-and-presets) first.

There are three kinds:

- **instrument rack**: chains of instruments (with their own effects), for MIDI tracks;
- **audio effect rack**: chains of audio effects;
- **MIDI effect rack**: chains of MIDI effects.

## making a rack

Two ways:

- Add **instrument rack**, **audio effect rack** or **MIDI effect rack** from the device library, like any device.
- Right-click the header of a device on a track and choose **group into instrument rack** (or **audio effect rack**, **MIDI effect rack**, depending on the device). The device moves into a new rack, on its first chain. This isn't offered for drum racks, or for devices that are already inside a rack or a drum pad.

A rack has no window: its chains and macros are on its card.

## chains

The rack's card lists one row per chain. A row shows the chain's name, then the same controls as a track header: **mute**, **solo**, **volume** and **pan**, a **zones** button, and **×** to delete the chain.

- Click a row's arrow to expand it and see its devices, in the same sections as a track's chain (MIDI effects, instrument, audio effects).
- **Add a chain**: in an instrument rack, **add instrument** creates a new chain with the instrument you pick. In an effect rack, **add chain** adds an empty chain.
- **Solo**: a click solos only this chain; [[key:Mod]]-click adds it to or removes it from the soloed chains.
- **Move a device** between chains: drag its card onto another chain's row.
- Double-click a chain's name, or right-click its row, to rename it. A chain without a name shows the name of its instrument or effects.

All chains receive the same input and play at the same time; their outputs are mixed.

### zones: keys, velocity and chain select

The **zones** button of a chain sets which notes it answers, with three ranges from 0 to 127:

- **keys**: the notes it plays. Give a bass chain the low keys and a lead chain the high keys to split the keyboard.
- **vel**: the velocities it plays. A soft layer for gentle notes, a harder one for loud notes.
- **chain select**: the values of the rack's **chain selector** that turn it on.

Every rack has a **chain selector** parameter (0 to 127). Give each chain a different chain select range, then turn the selector, automate it or map a macro to it, to switch between sounds with one control. By default every chain answers every key, velocity and selector value.

### what a rack can hold

- An audio effect rack takes audio effects only, and a MIDI effect rack MIDI effects only.
- An instrument rack takes anything, a drum rack included (unless the rack is itself on a drum pad).
- Racks can go inside racks, up to 6 levels deep. The deepest chains don't offer racks.

## macros

Every rack has 8 macros. Open the rack's parameter list (the sliders button on its header) to see them as knobs, with the chain selector.

To map a macro:

1. Drag the macro's small map handle onto a parameter of a device inside the rack, or click the handle and then click the parameter. You can also right-click the parameter and choose **assign to macro**, then the macro (in a rack inside another rack, first the rack).
2. The mapping shows on that parameter as a coloured ring and a depth chip, as for [modulation](/docs/making-music/modulation-and-sidechain#depth-rings-and-chips). Drag the chip to set how far the macro moves it.

One macro can drive many parameters, each with its own depth. For example, one "brightness" macro can open a filter and add some drive at the same time. A macro can also drive the rack's own chain selector. Macros can be automated and mapped to a MIDI controller like any knob.

A macro can also **switch a device on and off**: drop it on the device's power button (or right-click the button and choose **assign to macro**). An on device turns off when the macro passes the middle, an off one turns on; right-click the coloured dot by the button to invert or remove it. The button shows the device's live state. Modulators can switch their own device the same way.

## rack presets

Saving a preset on a rack (with its **presets** button) stores the whole rack: its chains, their devices and settings, its modulators and macro mappings. Loading it rebuilds all of that, as one undo step. This is how you keep a layered sound or an effect chain for other projects.

## where next

- [modulation and sidechain](/docs/making-music/modulation-and-sidechain): LFOs and envelopes inside a rack.
- [the drum rack](/docs/making-music/drum-rack): a rack with one chain per note.
- [devices and presets](/docs/making-music/devices-and-presets#presets): presets.
