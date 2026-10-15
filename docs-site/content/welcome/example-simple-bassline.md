---
title: example: simple bassline
order: 43
summary: Rebuild the simple bassline example from an empty project: an eighth-note acid bass over a kick and hats.
platform: all
audience: beginner
covers:
---

The **simple bassline** is one of the example loops on the welcome screen: 120 bpm, two bars, an acid bassline over the simplest beat, made only with the instruments that come with Ethereal. On the welcome screen, under **examples**, click **simple bassline** to open the finished loop, or **show me how** for a guided tour of it. This page rebuilds it from an empty project, step by step.

You need nothing else: the drum sounds are **poly synth** presets, so everything works in the desktop app and in the web version.

## 1. start a project and set the tempo

1. On the welcome screen, click **new project**. The project comes with a `synth` track: click its header and press [[action:edit.delete]], this guide makes its own tracks.
2. A new project already plays at 120 bpm: nothing to change. Other tempos go in the **bpm** field of the top bar (see [tempo and time signature](/docs/basics/tempo-and-time-signature)).
3. Turn on the **loop** button in the top bar, then drag the end of the loop brace above the tracks to the end of bar 2. See [playing and moving around](/docs/basics/playing-and-navigating#looping-a-section).

## 2. make the beat

1. Click **add track** above the track headers and choose **MIDI track**. The instrument library opens: pick **drum rack**. Double-click the track's name and call it `drums`.
2. Click the pad **C1**. Under the grid, the pad's chain has an instrument slot: click **add instrument**, pick **poly synth**, then open **presets** on its card and choose `kick`. Do the same for **F#1** with `closed hat`. More in [the drum rack](/docs/making-music/drum-rack#pads).
3. Double-click the empty lane of the `drums` track at the start of bar 1 and keep holding the second press, then drag to the end of bar 2: a two-bar clip appears (see [clips](/docs/making-music/clips)).
4. Click the clip: it opens in the **beat editor**. Draw this pattern in both bars (`x` is a hit, `.` a rest; each group of four cells is one beat):

```
kick        x . . .  x . . .  x . . .  x . . .
closed hat  . . x .  . . x .  . . x .  . . x .
```

## 3. add the bass

1. Click **add track**, choose **MIDI track**, and pick **poly synth** in the instrument library. Call the track `bass`.
2. Hover the poly synth's card in the inspector, open **presets** and choose **acid bass**: a saw wave through a resonant filter that snaps open on each note. See [instruments](/docs/making-music/instruments).

## 4. write the bassline

Make a two-bar clip on the `bass` track like in step 2. It opens in the piano roll. Draw these notes (positions are bar.beat.16th, as the top bar shows them; an 8th note is two 16ths long; C3 is middle C):

| starts at | length | note |
|---|---|---|
| 1.1.1 | 8th | E1 |
| 1.1.4 | 16th | E1 |
| 1.2.3 | 8th | G1 |
| 1.3.1 | 8th | E1 |
| 1.3.4 | 16th | E1 |
| 1.4.1 | 8th | A1 |
| 1.4.3 | 8th | G1 |
| 2.1.1 | 8th | E1 |
| 2.1.4 | 16th | E1 |
| 2.2.3 | 8th | B1 |
| 2.3.1 | 8th | A1 |
| 2.3.3 | 8th | G1 |
| 2.4.1 | 8th | D1 |
| 2.4.3 | 8th | E1 |

Mostly one low note, E1, with a few higher ones answering it. More in [the piano roll](/docs/making-music/piano-roll).

## 5. balance and play

In the track headers, turn the `drums` volume down to about -4 dB and the `bass` to about -4 dB (see [mixing](/docs/making-music/mixing)). Press [[action:transport.play]]. Then make it yours: drag notes up or down, you hear them as you move them, or try the **reese bass** preset.

## where next

- [House beat](/docs/welcome/example-house-beat): a full house kit with an off-beat bass.
- [The piano roll](/docs/making-music/piano-roll): select, move and quantize notes.
- [Automation](/docs/making-music/automation): open the filter over the bars.
