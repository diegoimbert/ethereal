---
title: example: lo-fi chords
order: 42
summary: Rebuild the lo-fi chords example from an empty project: four seventh chords through chorus, bit crushing and reverb, over a lazy beat.
platform: all
audience: beginner
covers:
---

The **lo-fi chords** loop is one of the example loops on the welcome screen: 80 bpm, four bars, four warm chords made dusty by three effects, over a lazy swung beat. Everything comes with Ethereal. On the welcome screen, under **examples**, click **lo-fi chords** to open the finished loop, or **show me how** for a guided tour of it. This page rebuilds it from an empty project, step by step.

You need nothing else: the drum sounds are **poly synth** presets, so everything works in the desktop app and in the web version.

## 1. start a project and set the tempo

1. On the welcome screen, click **new project**. The project comes with a `synth` track: click its header and press [[action:edit.delete]], this guide makes its own tracks.
2. In the top bar, click the **bpm** field, type `80` and press [[key:Enter]] (see [tempo and time signature](/docs/basics/tempo-and-time-signature)).
3. Turn on the **loop** button in the top bar. The loop brace above the tracks covers four bars in a new project, which is what this loop needs. See [playing and moving around](/docs/basics/playing-and-navigating#looping-a-section).

## 2. make the chords sound

1. Click **add track** above the track headers and choose **MIDI track**. The instrument library opens: pick **poly synth**. Double-click the track's name and call it `chords`.
2. Hover the poly synth's card in the inspector, open **presets** and choose **soft keys**.
3. Under the instrument, click **add audio effect** three times to add, in this order, each with its preset:

| effect | preset |
|---|---|
| chorus | classic chorus |
| bitcrusher | lo-fi sampler |
| reverb | plate |

The sound goes through them from top to bottom: the chorus widens and wobbles it, the bitcrusher wears it down like an old sampler, the reverb puts it in a room. See [effects](/docs/making-music/effects).

## 3. write the chords

Double-click the empty lane of the `chords` track at the start of bar 1 and keep holding the second press, then drag to the end of bar 4: a four-bar clip appears (see [clips](/docs/making-music/clips)). Click it to open it in the piano roll, and draw one chord per bar, each held for 3 and three quarter beats (C3 is middle C):

| bar | chord | notes |
|---|---|---|
| 1 | F major 7 | F2, A2, C3, E3 |
| 2 | E minor 7 | E2, G2, B2, D3 |
| 3 | D minor 7 | D2, F2, A2, C3 |
| 4 | C major 7 | C2, E2, G2, B2 |

Each chord is the one before moved one step down the scale. More in [the piano roll](/docs/making-music/piano-roll).

## 4. add a lazy beat

1. Click **add track**, choose **MIDI track**, and pick **drum rack**. Call the track `drums`.
2. Click the pad **C1**, click **add instrument** in the pad's chain under the grid, pick **poly synth**, then open **presets** on its card and choose `kick`. Do the same for **D1** with `snare` and **F#1** with `closed hat`. More in [the drum rack](/docs/making-music/drum-rack#pads).
3. Make a four-bar clip on the `drums` track and click it: it opens in the **beat editor**. Draw this pattern in each of the four bars (`x` is a hit, `o` a softer hit, `.` a rest; each group of four cells is one beat):

```
kick        x . . .  . . . .  . . x .  . . . .
snare       . . . .  x . . .  . . . .  x . . .
closed hat  o . o .  o . o .  o . o .  o . o o
```

The soft hats are optional: open the **piano roll** tab and lower their velocity (see [the piano roll](/docs/making-music/piano-roll)).

## 5. swing and balance

1. Select a clip: the inspector shows a **groove** row. Set the swing to **35%** on a **1/16** grid. See [groove, swing and humanize](/docs/advanced/groove#project-swing-vs-quantize-swing).
2. In the track headers, turn the `chords` volume up to about +4 dB and the `drums` down to about -8 dB, so the chords lead (see [mixing](/docs/making-music/mixing)).
3. Press [[action:transport.play]].

Then make it yours: try other chords, another preset on the bitcrusher, or more reverb.

## where next

- [Simple bassline](/docs/welcome/example-simple-bassline): an acid bass over a kick.
- [Effects](/docs/making-music/effects): every built-in effect.
- [Devices and presets](/docs/making-music/devices-and-presets#presets): save your own version of the sound.
