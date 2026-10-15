---
title: example: house beat
order: 40
summary: Rebuild the house beat example from an empty project: a drum rack kit, a four-on-the-floor beat and an off-beat bass.
platform: all
audience: beginner
covers:
---

The **house beat** is one of the example loops on the welcome screen: 124 bpm, two bars, a drum kit and a bass, made only with the instruments that come with Ethereal. On the welcome screen, under **examples**, click **house beat** to open the finished loop, or **show me how** for a guided tour of it. This page rebuilds it from an empty project, step by step.

You need nothing else: the drum sounds are **poly synth** presets, so everything works in the desktop app and in the web version.

## 1. start a project and set the tempo

1. On the welcome screen, click **new project**. The project comes with a `synth` track: click its header and press [[action:edit.delete]], this guide makes its own tracks.
2. In the top bar, click the **bpm** field, type `124` and press [[key:Enter]] (see [tempo and time signature](/docs/basics/tempo-and-time-signature)).
3. Turn on the **loop** button in the top bar, then drag the end of the loop brace above the tracks to the end of bar 2. See [playing and moving around](/docs/basics/playing-and-navigating#looping-a-section).

## 2. make the drum kit

1. Click **add track** above the track headers and choose **MIDI track**. The instrument library opens: pick **drum rack**. Its window opens with an empty grid of pads.
2. Double-click the new track's name and call it `drums` (any name works).
3. Click the pad **C1**. Under the grid, the pad's chain has an instrument slot: click **add instrument**, pick **poly synth**, then open **presets** on the poly synth's card and choose `kick`.
4. Do the same for the other sounds:

| pad | preset |
|---|---|
| C1 | kick |
| D#1 | clap |
| F#1 | closed hat |
| A#1 | open hat |

Press and hold a pad to hear it. More about pads in [the drum rack](/docs/making-music/drum-rack#pads).

## 3. draw the beat

1. Double-click the empty lane of the `drums` track at the start of bar 1 and keep holding the second press, then drag to the end of bar 2: a two-bar clip appears (see [clips](/docs/making-music/clips)).
2. Click the clip. It opens in the **beat editor**: one row per pad, one cell per 16th note. Click a cell to add a hit, click it again to remove it.
3. Draw this pattern in both bars (`x` is a hit, `o` a softer hit, `.` a rest; each group of four cells is one beat):

```
kick        x . . .  x . . .  x . . .  x . . .
clap        . . . .  x . . .  . . . .  x . . .
closed hat  . o . o  . o . o  . o . o  . o . o
open hat    . . x .  . . x .  . . x .  . . x .
```

The softer hats are optional. To make them, open the **piano roll** tab and lower their velocity (see [the piano roll](/docs/making-music/piano-roll)).

## 4. add the bass

1. Click **add track**, choose **MIDI track**, and pick **poly synth** in the instrument library. Call the track `bass`.
2. Hover the poly synth's card in the inspector, open **presets** and choose **deep sub**.

## 5. write the bass line

Make a two-bar clip on the `bass` track like in step 3. It opens in the piano roll. Draw these notes, each a 16th long (positions are bar.beat.16th, as the top bar shows them; C3 is middle C):

| starts at | note |
|---|---|
| 1.1.3, 1.2.3, 1.3.3, 1.4.3 | A1 |
| 2.1.3, 2.2.3 | F1 |
| 2.3.3, 2.4.3 | G1 |

Every note sits on an off-beat, between two kicks. More in [the piano roll](/docs/making-music/piano-roll).

## 6. balance and play

Press [[action:transport.play]]. Turn the `drums` track's volume down to about -3 dB and the `bass` to about -4 dB in their headers, so the mix doesn't clip (see [mixing](/docs/making-music/mixing)). Then make it yours: move a clap, try another bass preset, or add a third bar.

## where next

- [Hip-hop loop](/docs/welcome/example-hip-hop-loop): a swung beat with chords.
- [The drum rack](/docs/making-music/drum-rack): pads, choke groups and effects per pad.
- [Instruments](/docs/making-music/instruments): what the poly synth can do.
