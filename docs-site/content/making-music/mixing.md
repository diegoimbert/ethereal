---
title: mixing
order: 120
summary: Balance your tracks with volume, pan, mute and solo, route them to groups and return tracks, and use sends.
platform: all
audience: everyone
covers:
---

Mixing is setting how loud each track is, where it sits from left to right, and which effects it shares with other tracks. This page covers the mixer controls of a track and how audio flows from tracks to the master. You need a few tracks with sound in them.

> [!note]
> On a computer there is no separate mixer window: the mixer controls are in each track's header and in the inspector. The mobile beta has its own mixer view.

## a track's mixer

Every track has its mixer controls in two places: compact ones in its header in the arrangement, and bigger ones in the **mixer** section of the inspector when the track is selected.

- **volume**: from silence up to +6 dB. 0 dB leaves the level unchanged. Double-click to reset it to 0 dB.
- **pan**: left to right. Double-click to center it.
- **mute** turns the track off.
- **solo** plays only this track (and other soloed tracks). A click solos only this track; [[key:Mod]]-click adds it to the soloed tracks or removes it.
- The **meter** in the track header shows the track's level as it plays.

All of these can be automated (see [automation](/docs/making-music/automation)) and mapped to a MIDI controller.

## where the sound goes

Every track sends its audio to an **output**, chosen in the inspector's **output** menu:

- by default, to the **master** track, or to its group if the track is inside a [group](/docs/making-music/tracks);
- to a **group** or **return** track of your choice;
- to **no output**: the track isn't heard, but it can still key a sidechain.

The **master** track is the end of the line: everything you hear and export goes through it. It has volume, pan and mute, and its own devices (a limiter is a common last device), but no solo and no output menu.

### groups as buses

When you group tracks (select them and press [[action:track.group]]), the group track becomes a bus: its children play through it. The group's volume, pan and devices act on all of them together, for example one compressor on all your drums.

## sends and return tracks

A **return track** holds an effect that several tracks share, such as one reverb for the whole song. Each track sends some of its sound to it, with a **send** knob.

- To add a return track, open the **add track** menu and choose **return track**. It is also in a track's right-click menu (**add return track**), in the menu of the empty space below the tracks, and in the command palette ([[action:palette.open]]). On the mobile beta, it's in the track sheet's actions. A new return track goes right after the selected return, or below the other returns, above the master. You can also drag **return track** out of the **add track** menu and drop it between two returns. Put a reverb or a delay on it.
- To reorder the return tracks, drag a return's header up or down between the other returns (see [moving tracks](/docs/making-music/tracks#moving-tracks)). On the mobile beta, use **move up** and **move down** in its track sheet. Every send keeps going to the same return.
- When a project has return tracks, the inspector's mixer section shows one **send** knob per return track, in the returns' order. Sends start at silence: turn one up to send this track to that return. The first turn creates the send.
- Under each send knob, a small **post** / **pre** button sets where the send is taken from. **post** (the default) takes it after the track's volume (post-fader), so turning a track down also turns down what it sends. **pre** takes it before the volume (pre-fader), so the send stays at its level whatever the fader does, which is handy for a separate headphone mix or an effect that should keep going while the track fades out. The button is greyed out until the send exists. Switching it is one undo step.
- Send levels can be automated like volume.

## where next

- [effects](/docs/making-music/effects): the compressor, EQ and limiter for mixing.
- [modulation and sidechain](/docs/making-music/modulation-and-sidechain#sidechain): duck one track under another.
- [automation](/docs/making-music/automation): change the mix over time.
