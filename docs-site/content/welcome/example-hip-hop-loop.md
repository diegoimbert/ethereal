---
title: example: hip-hop loop
order: 41
summary: Rebuild the hip-hop loop example from an empty project: a swung boom-bap beat under two soft chords.
platform: all
audience: beginner
covers:
---

The **hip-hop loop** is one of the example loops on the welcome screen: 90 bpm, two bars, a swung drum beat and two soft seventh chords, made only with the instruments that come with Ethereal. On the welcome screen, under **examples**, click **hip-hop loop** to open the finished loop, or **show me how** for a guided tour of it. This page rebuilds it from an empty project, step by step.

You need nothing else: the drum sounds are **poly synth** presets, so everything works in the desktop app and in the web version.

## 1. start a project and set the tempo

1. On the welcome screen, click **new project**. The project comes with a `synth` track: click its header and press [[action:edit.delete]], this guide makes its own tracks.
2. In the top bar, click the **bpm** field, type `90` and press [[key:Enter]] (see [tempo and time signature](/docs/basics/tempo-and-time-signature)).
3. Turn on the **loop** button in the top bar, then drag the end of the loop brace above the tracks to the end of bar 2. See [playing and moving around](/docs/basics/playing-and-navigating#looping-a-section).

## 2. make the drum kit

1. Click **add track** above the track headers and choose **MIDI track**. The instrument library opens: pick **drum rack**. Its window opens with an empty grid of pads.
2. Double-click the new track's name and call it `drums`.
3. Click the pad **C1**. Under the grid, the pad's chain has an instrument slot: click **add instrument**, pick **poly synth**, then open **presets** on the poly synth's card and choose `kick`.
4. Do the same for the other sounds:

| pad | preset |
|---|---|
| C1 | kick |
| D1 | snare |
| F#1 | closed hat |

More about pads in [the drum rack](/docs/making-music/drum-rack#pads).

## 3. draw the beat

1. Double-click the empty lane of the `drums` track at the start of bar 1 and keep holding the second press, then drag to the end of bar 2: a two-bar clip appears (see [clips](/docs/making-music/clips)).
2. Click the clip. It opens in the **beat editor**: one row per pad, one cell per 16th note. Click a cell to add a hit, click it again to remove it.
3. Draw this pattern (`x` is a hit, `o` a softer hit, `.` a rest; each group of four cells is one beat). The kick changes in bar 2; the snare and the hats are the same in both bars:

```
bar 1
kick        x . . .  . . . x  . . x .  . . . .
snare       . . . .  x . . .  . . . .  x . . .
closed hat  x . o .  x . o .  x . o .  x . o .

bar 2
kick        x . x .  . . . .  . . x .  . . . .
snare       . . . .  x . . .  . . . .  x . . .
closed hat  x . o .  x . o .  x . o .  x . o .
```

The softer hats are optional: open the **piano roll** tab and lower their velocity (see [the piano roll](/docs/making-music/piano-roll)).

## 4. swing it

With the clip selected, the inspector shows a **groove** row: set the swing to **50%** on a **1/16** grid. Every MIDI clip of the project now plays its off-beat 16ths a little late, which gives the hats their bounce. Nothing moves in the clip: set it back to 0% for a straight beat. See [groove, swing and humanize](/docs/advanced/groove#project-swing-vs-quantize-swing).

## 5. add the keys

1. Click **add track**, choose **MIDI track**, and pick **poly synth** in the instrument library. Call the track `keys`.
2. Hover the poly synth's card in the inspector, open **presets** and choose **soft keys**.
3. Under the instrument, click **add audio effect** and pick **reverb**, then choose its **small room** preset. See [effects](/docs/making-music/effects).

## 6. write the chords

Make a two-bar clip on the `keys` track like in step 3. It opens in the piano roll. Draw two chords of four notes (C3 is middle C):

| starts at | length | notes |
|---|---|---|
| 1.1.1 | 3 and a half beats | C3, D#3, G3, A#3 |
| 2.1.1 | 3 and a half beats | G#2, C3, D#3, G3 |

The first is C minor 7, the second A-flat major 7. More in [the piano roll](/docs/making-music/piano-roll).

## 7. balance and play

Press [[action:transport.play]]. Turn the `drums` track's volume down to about -3 dB in its header, and leave the `keys` at 0 dB (see [mixing](/docs/making-music/mixing)). Then make it yours: move a kick, try another chord, or change the swing.

## where next

- [Lo-fi chords](/docs/welcome/example-lofi-chords): four chords through three effects.
- [Groove, swing and humanize](/docs/advanced/groove): more ways to loosen a beat.
- [Effects](/docs/making-music/effects): what else you can put after an instrument.
