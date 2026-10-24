---
title: effects
order: 70
summary: What each built-in audio and MIDI effect does, and the controls that matter most.
platform: all
audience: everyone
covers:
  - devices/compressor #compressor
  - devices/eq #eq
  - devices/delay #delay
  - devices/grain-delay #grain-delay
  - devices/reverb #reverb
  - devices/convolution-reverb #convolution-reverb
  - devices/limiter #limiter
  - devices/utility #utility
  - devices/color-effects-saturator-bitcrusher-auto-filter #color-saturator-bitcrusher-auto-filter
  - devices/modulation-effects-chorus-phaser-flanger-tremolo #modulation-chorus-phaser-flanger-tremolo
  - devices/shifter #shifter
  - devices/dynamics-gate-multiband-compressor-transient-shaper #dynamics-gate-multiband-compressor-transient-shaper
  - devices/analysis-spectrum-and-tuner #analysis-spectrum-and-tuner
  - devices/midi-effects-arpeggiator-chord-scale-note-length-velocity-ra #midi-effects
  - devices/external-instrument-external-audio-effect #external-audio-effect
---

Effects change a sound. **Audio effects** go after the instrument (or on an audio track) and process audio. **MIDI effects** go before the instrument on a MIDI track and change the notes it receives. Add them from the chain's add buttons (see [devices and presets](/docs/making-music/devices-and-presets#adding-a-device)). Every effect has factory presets, which are a good way to learn what it can do.

Most audio effects have a **mix** control (how much of the effect you hear against the original sound) and an **output** level.

## the essentials

These are the effects most songs use.

### eq

An 8-band equalizer to cut or boost frequencies.

- Each band is a low cut, low shelf, bell, notch, high shelf or high cut. Bands 2 to 7 start on, bands 1 and 8 (a low cut and a high cut) start off.
- **On the curve**: drag a band's handle to change its frequency (left and right) and gain (up and down). [[key:Alt]]-drag or scroll the wheel over it to change its width (Q). [[key:Shift]] makes any drag finer. Double-click a handle to turn the band on or off; right-click it to change its type or reset it.
- A focused handle answers the arrow keys: left and right for frequency, up and down for gain, [[key:Alt]] with up and down for Q.
- **channel**: process both channels, only the left or right, or only the mid or side of a stereo signal.
- The live spectrum behind the curve shows what goes in and comes out.

### compressor

Evens out the volume: when the sound goes over the **threshold**, it is turned down by the **ratio**.

- **attack** and **release**: how fast it reacts and lets go.
- **knee**: a soft knee starts compressing gradually around the threshold.
- **makeup** turns the result back up; **auto makeup** does it for you.
- **mix**: blend in the uncompressed sound (parallel compression).
- **detector**: **peak** reacts to every peak, **RMS** to the average level, and **expand** turns it into a downward expander that makes quiet parts quieter.
- **lookahead** (up to 10 ms) catches fast transients, and adds that much latency, which Ethereal compensates.
- It can be keyed by another track (see [sidechain](/docs/making-music/modulation-and-sidechain#sidechain)). **sidechain HPF** then filters the low end out of the key signal, so a kick's sub doesn't trigger it too much; **sidechain slope** makes that filter gentle (6 dB/oct) or steeper (12 dB/oct).

### reverb

An algorithmic reverb: the sound of a room or hall.

- **pre-delay**: time before the reverb starts. **size** and **decay** (0.2 to 20 seconds): how big the space is and how long the tail lasts.
- **damping** darkens the tail, **diffusion** makes it smoother, **width** sets its stereo size.
- **low cut** and **high cut** filter what goes into the reverb.
- **freeze** holds the tail forever.
- **shimmer** pitch-shifts the tail inside the reverb, so every echo rises again: **amount** sets how much, **pitch** by how many semitones (+12 an octave, +7 a fifth, -12 falls), **tone** darkens the shifted sound. Presets: *shimmer hall*, *octave cloud*.
- **mix** and **output**. Output sets the level of the reverb itself, not the dry sound.

### delay

Echoes.

- **time** in milliseconds (up to 2 seconds), or turn on **sync** to pick a note value (1/32 to a whole note, including dotted and triplet values) that follows the tempo.
- **feedback**: how many echoes. **mix**: how loud they are.
- **ping-pong**: echoes bounce between left and right (the input is summed to mono first).
- **low cut** and **high cut** act inside the feedback loop, so each echo gets darker or thinner than the one before.
- **mod rate** and **mod depth** wobble the delay time, like tape.
- **freeze** loops what is in the delay right now, without taking new input.

### grain delay

Echoes chopped into tiny overlapping grains, which you can transpose and scatter.

- **pitch** transposes the grains (-36 to +12 semitones); with **feedback**, each echo is transposed again, so the repeats climb or fall.
- **frequency** sets how many grains start per second, and so their size: low values give long, smooth grains (a simple pitch shifter), high values a buzzy, granular texture. Drag the pad: across for the **frequency**, up for the **pitch**.
- **spray** scatters each grain's start in time (a little smears the sound, a lot breaks it up and widens it); **random pitch** detunes each grain at random.
- **time** in milliseconds, or turn on **sync** for a note value that follows the tempo. **mix** blends the dry sound.

### limiter

A brickwall limiter, usually last on the master track: nothing goes above the **ceiling**.

- **gain** drives the input into it, to make the sound louder. **release** sets how fast it recovers.
- It always looks 5 ms ahead (that latency is compensated). It catches true peaks too (the peaks between samples that a converter or a streaming encoder rebuilds), so the ceiling holds after export.

### utility

Small but useful fixes: **gain** (±36 dB), **pan**, stereo **width** (0 to 200 %) or a mid/side balance, **invert** the left or right channel, **mono**, **bass mono** below a frequency you choose, a **channel** choice (stereo, left, right or swapped), a **DC filter** and **mute**. Its meters show the output level.

## space

### convolution reverb

A reverb made from a recording of a real space (an impulse response, or IR).

- Pick one of the 6 factory IRs (small room, chamber, plate, concert hall, cathedral, ambience), or drop your own audio file on it (only the first 10 seconds are used).
- **mix** starts at 30 %. **decay** shortens the IR and **size** stretches it. **reverse** plays it backwards.
- **pre-delay**, **low cut**, **high cut**, **width** and **gain**.
- In its window, drag the pre-delay and decay markers directly on the IR's picture.

## color: saturator, bitcrusher, auto filter

- **saturator**: adds warmth or distortion. Pick a **curve** (soft, hard, tube, tape, fold, rectify), then set **drive**, **bias** and **tone**. **auto gain** (on by default) keeps the level steady as you add drive. **oversampling** makes it cleaner. It adds a small, fixed latency.
- **bitcrusher**: lo-fi digital grit. **bits** lowers the resolution, **rate** the sample rate; **jitter** and **dither** roughen or smooth it.
- **erosion**: digital wear that follows the sound. It wobbles a very short delay with a **sine** (metallic sidebands) or with **noise** (fizz and hiss; **wide noise** is different on each side). Drag the pad: across for the **frequency**, up for the **amount**. **width** sets how wide the noise band is. Bright sounds erode first, and silence stays silent. It adds a tiny, fixed latency.
- **pedal**: a guitar pedal. Pick a **type**: **overdrive** (warm and smooth, it keeps the lows round), **distortion** (tight and aggressive) or **fuzz** (unstable: the end of each note sputters). **gain** sets how hard it drives; even at 0 % it colors the sound, and the level stays about the same as you turn it up, so use **output** to set the volume. **bass**, **mid** and **treble** shape the tone (**mid freq** picks 500 hz, 1 khz or 2 khz), **sub** adds weight below 250 hz, and **mix** blends in the dry sound. It runs oversampled, which adds a small, fixed latency.
- **auto filter**: a filter (7 types) that moves by itself. Its **envelope** follows the level of the sound (or of a sidechain, when one is set) to open the filter; its **LFO** sweeps it in a cycle, free or synced to the tempo, with a **stereo phase** offset between left and right. It runs oversampled, so **drive** stays clean; that adds a small, fixed latency.

## modulation: chorus, phaser, flanger, tremolo

These effects make the sound move with an LFO. Most have **sync** to lock their speed to the tempo; a synced effect follows the song position while it plays. Chorus, phaser and flanger start at 50 % mix.

- **chorus**: thickens a sound with slightly detuned copies. Its modes are chorus, ensemble and vibrato.
- **phaser**: a sweeping, hollow sound, with 2 to 12 stages.
- **flanger**: a jet-like sweep. **through zero** makes it more dramatic.
- **tremolo**: makes the volume pulse, or with **auto-pan**, moves the sound from left to right.

### shifter

Moves the pitch or the frequencies of a sound. Pick a **mode**:

- **pitch** transposes by **pitch** (-24 to +24 semitones) and **detune** (cents). **window** sets the length of the slices it is made of: longer for low notes and pads, shorter for high notes and drums.
- **frequency** moves every partial up or down by the same number of hertz (**shift**, plus **fine**). Harmonics stop being harmonic: a few hertz against the dry sound swirls like a phaser that never ends, hundreds of hertz sound metallic or bell-like.
- **ring** multiplies the sound by a **carrier** tone (plus **fine**): audio rates give robotic, clangorous sidebands; a few hertz under some dry sound give a choppy tremolo. **drive** distorts the result (by **amount**).
- the **lfo** moves the shift by itself: **lfo depth** in hertz (frequency and ring modes) or **lfo pitch** in semitones (pitch mode). Pick a **waveform** (sine, triangles, saws, rectangle with its **duty**, smooth or stepped random), a **rate** in hertz or **sync** it to the tempo. **stereo** offsets the right side by a **phase**, or makes it **spin** faster; **offset** moves the starting point, and **width** spreads the stepped random between the sides.
- the envelope **follower** moves it with the input's level: **env depth** in hertz, **env pitch** in semitones (negative values go down), with its **attack** and **release**.
- the **delay** comes after the shift: a **delay time** in milliseconds, or a **delay rate** with **delay sync**. Its **feedback** goes back through the shifter, so each repeat is shifted again (spirals in frequency mode, octave cascades in pitch mode); **tone** darkens the repeats.
- **wide** mirrors the right channel (the opposite detune or shift; in ring mode the carrier is a quarter cycle apart, so the tremolo alternates between the sides). **mix** blends the dry sound.

## dynamics: gate, multiband compressor, transient shaper

- **gate**: mutes the sound when it falls below the **threshold** (for noise or drum bleed). **mode** can make it a gentler expander; **flip** inverts it; **hold**, **lookahead** and a sidechain input are there for precise gating.
- **multiband compressor**: three compressors, one for the lows, mids and highs, split at two movable crossover frequencies. Each band can be soloed or bypassed. A sidechain keys all three bands.
- **transient shaper**: boosts or cuts the **attack** (the hit) and the **sustain** (the body) of sounds, independently of their level. **clip** keeps the output from going over.

## analysis: spectrum and tuner

These don't change the sound: the audio passes through untouched. Open their window to see what they show.

- **spectrum**: a frequency analyzer, with block size, averaging, channel (including mid and side), display range, slope and peak hold.
- **tuner**: shows the pitch of what you play. The **reference** goes from 415 to 466 Hz (440 by default). **mute output** silences the track while you tune.
- **loudness**: a loudness meter (the ebu r 128 way streaming services and broadcasters measure). It shows the **momentary** (last 0.4 s), **short-term** (last 3 s) and **integrated** loudness (the whole playback, in lufs, quiet parts left out), the **range** (how much the loudness varies, in lu) and the **true peak** (the highest point of the waveform, between the samples too, in dbtp; red above -1). The graph shows the history; the dashed line is your **target** (-14 by default). With **reset on play** on, every playback starts a new measurement. Put it last on the master to check a mix.

## midi effects

MIDI effects go on MIDI tracks, before the instrument.

| effect | what it does |
|---|---|
| **arpeggiator** | plays the notes you hold one after another: 9 styles (up, down, up-down, converge, as played, random, chord…), a rate, gate length, octaves and the distance between them, swing, **hold** and retrigger. It keeps running while the transport is stopped, so you can play it live. |
| **chord** | adds up to 6 extra notes at fixed intervals to every note, each with its own velocity, and can **strum** them |
| **scale** | moves every note into a scale: the track's, the project's, or a custom one. It can round to the nearest note, up or down. |
| **note length** | sets every note's length, in time or synced to the tempo |
| **velocity** | reshapes velocities with a curve, an output range and some randomness |
| **random** | randomly changes pitches (optionally within the scale), velocity, timing and length. It's driven by a **seed**, so the same seed gives the same result every time you play or export. |
| **pitch** | transposes notes, and can block, fold or limit them to a key range |
| **note blocker** | drops every note, so nothing after it plays: like muting the MIDI going in, while keeping the chain. CC, pitch bend and aftertouch still pass unless **block everything** is on. Notes held when you add it or switch it on stop, and switching it off doesn't replay them: they come back with the next note. |

## external audio effect

Sends a track's audio out to a hardware effect and brings it back [[desktop]].

- In its window, pick the **audio outputs** the effect's input is plugged into and the **audio inputs** it comes back on.
- **send gain**, **return gain**, **mix** and **invert phase**.
- **measure** times the round trip and sets the **latency**, so the hardware stays in time.
- Missing ports stay selected but are marked missing, and the device is silent until they come back. In the web app, the routing can't be changed.

## where next

- [modulation and sidechain](/docs/making-music/modulation-and-sidechain): duck a bass under the kick, or move parameters with an LFO.
- [racks and macros](/docs/making-music/racks-and-macros): combine several effects into one device.
- [mixing](/docs/making-music/mixing): volume, pan and return tracks for shared reverbs.
