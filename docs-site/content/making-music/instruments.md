---
title: instruments
order: 60
summary: What each built-in instrument does and what its main controls change.
platform: all
audience: everyone
covers:
  - devices/synth #synth
  - devices/poly-synth #poly-synth
  - devices/fm-synth #fm-synth
  - devices/sampler #sampler
  - devices/sampler-slice-mode #slice-mode
  - devices/multisampler #multisampler
  - devices/external-instrument-external-audio-effect #external-instrument
---

An instrument turns the notes of a MIDI track into sound. Each MIDI track has one instrument, at the middle of its chain (see [devices and presets](/docs/making-music/devices-and-presets)). This page goes through the built-in instruments, from the simplest to the most specialized. Every one of them has factory presets: open the **presets** button on its header to try them before you start turning knobs.

| instrument | good for |
|---|---|
| **synth** | a quick bass, lead or pad from one simple oscillator |
| **poly synth** | richer synth sounds: two oscillators, wavetables, unison, LFOs |
| **fm synth** | bells, electric pianos, metallic and glassy tones |
| **sampler** | playing one audio file, as a one-shot or across the keyboard |
| **multisampler** | real instruments recorded note by note |
| **drum rack** | drum kits, one sound per pad (see [the drum rack](/docs/making-music/drum-rack)) |
| **external instrument** | a hardware synth connected to your computer [[desktop]] |

## synth

A simple polyphonic synth (up to 16 notes at once) with one oscillator.

- **waveform**: sine, saw, square or triangle. Saw and square sound bright, sine and triangle soft.
- **transpose** and **fine**: pitch in semitones and in cents.
- **attack**, **decay**, **sustain**, **release**: the volume envelope of each note.
- **cutoff** and **resonance**: a low-pass filter. Lower the cutoff for a darker sound.
- **env amount**: lets the volume envelope sweep the cutoff (up to six octaves either way), for plucks and wah-like attacks. **key tracking** opens the filter more on higher notes.
- **glide**: slides the pitch from one note to the next.
- **velocity**: how much playing harder makes a note louder.
- **mode**: **poly** plays chords; **mono** plays one note at a time; **legato** plays one note at a time and glides only when notes overlap. In mono and legato, releasing the newest key goes back to the last key still held.

## poly synth

A fuller synth with two oscillators, a sub oscillator and noise.

- **osc 1** and **osc 2**: saw, square, triangle, sine or **wavetable**. A wavetable oscillator uses one of 8 tables (basic shapes, harmonic sweep, PWM, formant, digital, organ, vocal, metallic), and **position** morphs through it. Each oscillator has octave, semitones, fine, pulse width and level. Osc 2 starts silent: raise its level to hear it.
- **sub / noise**: a sub oscillator one or two octaves down, and noise with a color control. Both start silent.
- **filter**: low-pass 24 or 12, high-pass, band-pass or notch, with cutoff, resonance, drive, key tracking and env amount.
- **amp**, **filter** and **mod envelopes**. The mod envelope can be sent to pitch, osc 2 pitch, wavetable position, pulse width or cutoff.
- **LFO 1** and **LFO 2**: each has a shape, a rate (or a synced rate), a target (pitch, cutoff, amp, pan, wavetable position or pulse width) and an amount.
- **unison**: up to 16 stacked voices per note, with detune (in cents), blend and stereo spread, for wide, thick sounds.
- **voice**: number of voices, poly / mono / legato mode, glide, pitch bend range and **drift** (a slight analog-style instability).

## fm synth

Four operators, **A** to **D**, that modulate each other's frequency. The result can be anything from soft bells to harsh metallic tones.

- **algorithm**: how the operators are connected (11 choices). Click one of the algorithm diagrams in the window to pick it. The operators that reach the output are heard; the ones that feed into them change their tone. For example, `D→C→B→A` is a single stack where only A is heard, and `A+B+C+D` plays all four side by side.
- each operator has a **wave**, a frequency **ratio** (coarse and fine) or a **fixed** frequency, a **level**, its own envelope, **velocity** sensitivity and **feedback** (it modulates itself).
- only operator A is audible at first: the other operators start at the lowest level. Raise B, C or D to hear the FM effect.
- a **filter** (off by default) with its own envelope, one **LFO** that can go to pitch, filter or volume, **tone**, and **antialias** (2× oversampling, on by default, for cleaner high notes).

## sampler

Plays one audio file.

- The first time you open a sampler's window, a sample picker opens: search your library and pick a sample, or drag one from the [browser](/docs/sounds-and-plugins/browser) onto the sampler.
- **mode**: **one-shot** plays the whole sample at its own pitch on every note and ignores note releases (good for drum hits); **pitched** plays it higher or lower depending on the key, relative to the **root key**, and stops when you release the note.
- **start** and **end** choose the part of the sample that plays; **reverse** plays it backwards.
- **loop** (pitched mode): forward or ping-pong between loop start and loop end, with a **crossfade** to hide the click at the loop point.
- a **filter** (low-pass, high-pass or band-pass), an envelope, velocity sensitivity and up to 32 voices.

### slice mode

The switch at the right of the sample, **classic** | **slice**, chooses how the sampler plays. In **classic** mode every key plays the start to end range. In **slice** mode the sample is cut into **slices**, and each key plays one of them: the **first key** (C1 by default) plays slice 1, the next key slice 2, and so on. Other keys are silent. This is how you play a drum loop's hits from a keyboard or a MIDI clip.

- Switching to **slice** on a sample without slices finds its transients (the start of each hit) right away. The slices replace the waveform, each marker labelled with its key.
- **auto-slice** cuts the sample again: by **transients** (raise the **sensitivity** for more slices), on a **beat grid** (1/16 to 1 bar, at the project tempo) or in **equal parts**.
- **Double-click** the waveform to add a slice, **drag** a marker to move it, and **double-click** a marker (or select it and press [[key:Delete]], or right-click it) to remove it.
- **Press** a slice to hear it: it plays through the track, like a note.
- **slice to drum rack** replaces the sampler by a [drum rack](/docs/making-music/drum-rack#slicing-a-sample-to-pads) with one pad per slice, and opens its window.

Every slicing action is one undo step.

## multisampler

Plays different samples on different keys and velocities, like a sampled piano.

- Its window shows a map of **zones**: keys left to right, velocity bottom to top. Each zone is one sample with its key range, velocity range, root key, tune, gain, pan and loop.
- **Drop several audio files** from your computer onto a key: they become zones spread across the keyboard. Root notes are read from file names such as `C3` or `60`. A single sample can also come from the browser.
- Click a zone to select it and edit it in the zone inspector, where you can type values. Drag a zone's body to move it, or its edges to change its key or velocity range. [[key:Delete]] removes the selected zone.
- **round robin**: when several zones cover the same note, **cycle** plays them in turn and **random** picks one, so repeated notes don't sound identical.
- The rest is shared by all zones: volume, pan, transpose, an amp envelope, a filter with its own envelope, glide and up to 64 voices.

## external instrument

Plays a hardware synth from Ethereal [[desktop]]: notes go out to the synth through a MIDI port, and its audio comes back in through your audio interface's inputs.

- In its window, choose the **MIDI output** port and channel, and the **audio inputs** the synth is plugged into.
- **measure** times the round trip from Ethereal to the synth and back, and sets the **latency** so the synth stays in time with the rest of the song.
- **gain** sets the level of the returning audio.
- If you open the project on a computer without those ports, they stay selected but are marked missing, and the device is silent until they come back.
- In the web app, the routing is shown but can't be changed.

The **external audio effect** does the same for a hardware effect: see [effects](/docs/making-music/effects#external-audio-effect).

## where next

- [effects](/docs/making-music/effects): shape the sound of your instruments.
- [the drum rack](/docs/making-music/drum-rack): build a kit from samples.
- [modulation and sidechain](/docs/making-music/modulation-and-sidechain): make parameters move by themselves.
