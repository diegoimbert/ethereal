---
title: takes and comping
order: 150
summary: Find your recorded takes and build one performance from the best parts of each.
platform: all
audience: everyone
covers:
  - clips/takes-where-they-come-from #where-takes-come-from
  - clips/swipe-comping-and-choosing-takes #comping-picking-the-best-parts
  - clips/take-lane-menu-crossfades-promote-flatten #the-take-lane-menu
---

When you record the same part several times, every pass is kept as a **take**. Comping (short for compiling) means building one performance from the best parts of each take: the verse from take 2, the last line from take 4.

## where takes come from

Takes are made automatically when you [record](/docs/making-music/recording):

- several passes of the loop,
- with **punch** on,
- over clips that are already there.

Each pass goes on its own **take lane** under the track, and the newest one plays. Clips that were already in the recorded range move to the first take lane, so nothing is lost.

A track with takes shows a takes button on its header, with the number of takes. Click it to show or hide the take lanes. You can also right-click the track header and choose **show take lanes**, or **new take lane** to add an empty lane by hand.

## comping: picking the best parts

Each take lane draws its clips, and highlights the ranges that currently play from that take.

- **Swipe**: drag across a take lane. When you release, that range of the track plays from this take. The range snaps to the grid ([[key:Alt]]: no snapping).
- **Click** a take lane to pick that take for the part under the pointer. That part is then selected.
- With a part selected, [[action:take.previous]] and [[action:take.next]] step through the takes for it, so you can compare them quickly.
- The headphones button on a lane's header **auditions** that take: it plays that take alone, so you can listen to it whole.

Every comp change is one step in the [undo history](/docs/basics/undo-and-history).

## the take lane menu

Right-click a take lane for more:

| item | what it does |
|---|---|
| **audition take** / **stop auditioning** | plays this take alone |
| **comp whole take** | the whole span of this take plays from it |
| **promote clip to main lane** | moves the clip under the pointer out of the takes, onto the track |
| **remove from comp** | the part under the pointer no longer plays from this take |
| **no crossfade**, **crossfade 5 ms**, **20 ms**, **100 ms** | the crossfade at the edges of the part under the pointer, to hide clicks where takes meet |
| **rename** | renames the take |
| **move up** / **move down** | reorders the take lanes |
| **delete take** | removes the take |

### flattening

Right-click the track header for two more choices, once the track has a comp:

- **flatten comp** bakes the comp into normal clips on the track and keeps the take lanes.
- **flatten comp and delete takes** bakes the comp and deletes every take. It asks first.

> [!warning]
> **Flatten comp and delete takes** throws the other takes away. You can undo it right away, but once it is out of the undo history the takes are gone. Use **flatten comp** if you might want them later.

## where next

- [Recording](/docs/making-music/recording): record more takes.
- [Audio clips](/docs/making-music/audio-clips): fades and crossfades on the result.
- [Freeze, flatten and bounce](/docs/advanced/freeze-and-bounce): commit a track to audio.
