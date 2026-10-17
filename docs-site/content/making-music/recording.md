---
title: recording
order: 140
summary: Arm tracks, choose inputs and monitoring, use count-in and punch, and record audio and MIDI.
platform: desktop
audience: beginner
covers:
  - recording/recording-needs-the-desktop-app #recording-needs-the-desktop-app
  - recording/arm-a-track-then-record #your-first-recording
  - recording/arming-is-exclusive-cmd-ctrl-click-arms-several #arming-several-tracks
  - recording/arm-state-isn-t-saved-with-the-project #arming-several-tracks
  - recording/arming-an-audio-track-with-no-input-device-opens-settings #your-first-recording
  - recording/the-in-panel-input-device-per-track-arm-input-and-monitor #the-in-panel
  - recording/monitor-modes-auto-in-off #monitoring
  - recording/midi-input-port-and-channel-filters #midi-input-filters
  - recording/count-in-pre-roll #count-in
  - recording/punch-records-only-inside-the-loop-region #punch-in-and-out
  - recording/recording-over-clips-or-in-a-loop-makes-takes-not-overdub #recording-over-clips-and-in-a-loop
  - recording/takes-show-lanes-swipe-to-comp-up-down-to-switch #recording-over-clips-and-in-a-loop
  - recording/live-view-while-recording #while-you-record
  - recording/silent-take-warning-macos-microphone-permission #a-silent-take
  - recording/switching-projects-while-recording-loses-the-recording #while-you-record
  - recording/recording-another-track-resampling #recording-another-track
---

This page shows how to record audio (a voice, a guitar) and MIDI (a keyboard or controller) onto tracks. You need an audio track or a MIDI track (see [tracks](/docs/making-music/tracks)). For audio, set up your input device first in [audio and MIDI setup](/docs/making-music/audio-and-midi-setup).

## recording needs the desktop app [[web]]

In the browser, the record and **punch** buttons are disabled, and the inputs panel says that recording needs the desktop app: the browser gives Ethereal no access to audio or MIDI inputs. Two things still work on the web:

- the [computer MIDI keyboard](/docs/making-music/computer-midi-keyboard), to play notes,
- [capture MIDI](/docs/making-music/capture-midi), to turn what you just played into a clip.

[Get the desktop app](https://github.com/diegoimbert/ethereal/releases) to record audio or hardware MIDI.

## your first recording

1. Click **●** on a track's header to **arm** it: the track is ready to record.
2. Press the record button (**●**) in the transport bar. Playback starts (after the [count-in](#count-in), if you set one) and the transport shows **rec**.
3. Play or sing.
4. Stop, with the stop button or [[action:transport.play]].

Everything recorded becomes clips, as one step in the [undo history](/docs/basics/undo-and-history): one undo removes the whole recording.

If you arm an audio track and no input device is open yet, **settings** opens on the **audio** tab and asks you to choose an input device. Without one, the recording would be silent.

Record has no default shortcut. You can give it one in [keyboard shortcuts](/docs/reference/keyboard-shortcuts). The **ableton-like** preset uses [[key:F9]].

### arming several tracks

A plain click on a track's **●** arms that track and disarms all the others. To record several tracks at once, [[key:Mod]]-click their **●** buttons to add them. In the **in ▾** panel (below), the arm buttons never disarm other tracks.

Which tracks are armed is remembered on this computer for each project: when you reopen a project, the same tracks are armed again. Arming is not saved in the project file (it doesn't travel with it) and is not part of undo.

## the in panel

The **in ▾** button next to the record controls opens the inputs panel. It has:

- **audio input**: the input device (the same choice as in the audio settings).
- a table with every audio and MIDI track, with three columns:
  - **arm**: arms the track without disarming the others.
  - **input**: what the track records from. For an audio track: **no input**, each mono channel of your interface, or a stereo pair such as **1/2**. For a MIDI track: **all ports** or one MIDI port, plus a channel filter.
  - **monitor**: see below.

### monitoring

Monitoring means hearing the input through the track (and its effects) while you play.

| mode | what you hear |
|---|---|
| **auto** (the default) | the input while the track is armed |
| **in** | the input all the time, armed or not |
| **off** | never the input |

> [!warning]
> Monitoring a microphone through speakers can cause feedback, a loud howl. Use headphones, or set the track's monitoring to **off** and listen to the source directly.

### midi input filters

A MIDI track's input is **all ports** by default, with **all channels**. Each armed MIDI track records everything its filter accepts, so with the defaults every armed MIDI track records every controller.

To record two controllers (or two MIDI channels) onto two tracks at once, set each track's input to one port, and choose a channel (**ch 1** to **ch 16**) if needed.

## count-in

The **count-in** select (off, 1, 2 or 4 bars) starts playback that many bars before the record position, so you have time to get ready. The setting is saved with the project. Nothing you play during the count-in is kept, so a note you play early is lost.

## punch in and out

With **punch** on, only what you play inside the loop region is kept. The loop brace on the ruler is the punch range: there are no separate punch markers. Each pass through the loop becomes a [take](/docs/making-music/takes-and-comping).

Punch is remembered on this computer for each project, like the armed tracks: it is on again when you reopen a project you left with punch on.

## recording over clips and in a loop

Recording never mixes new material into an existing clip, and there is no MIDI overdub mode. Instead, takes are made:

- when you record over existing clips,
- when you record several passes of the loop,
- when you punch in.

Each pass becomes a take, on its own take lane, and the newest one plays. Existing clips in the recorded range move to the first take lane, so they are not lost. Click the takes button on the track's header to show the take lanes. See [takes and comping](/docs/making-music/takes-and-comping) to choose the best parts.

> [!note]
> If you come from Ableton Live and expect MIDI overdub, record a new pass and then comp, or record onto another track.

## while you record

While recording, a growing "recording" clip shows the waveform or the notes as they arrive, and held notes grow as you hold them. A flat waveform is the first sign of a wrong input or channel. When you stop, it is replaced by the real clips in the same place.

If you open or create another project while recording, the recording stops first and the takes go into the project you recorded in, which is saved before the other one opens.

### a silent take

If a take is completely silent for at least a second, a warning appears. On macOS, the most common cause is that the system denied Ethereal access to the microphone: allow it in **System Settings › Privacy & Security › Microphone**. Also check the input device and the track's input channels.

## recording another track

An audio track can record the output of another track, for example to resample a synth with its effects. Select the track, then in the inspector set **input** to **from** the other track. A second select chooses where the signal is taken: **pre fx**, **post fx** or **post mixer**. Choosing a track also turns the recording track's monitoring to **in**, so you hear it.

The recording is aligned with that track's own latency. This works on the desktop app only. In the browser, arming such a track is disabled.

## where next

- [Takes and comping](/docs/making-music/takes-and-comping): pick the best parts of several takes.
- [Capture MIDI](/docs/making-music/capture-midi): keep what you played without recording.
- [Audio clips](/docs/making-music/audio-clips): fades, warp and reverse on what you recorded.
