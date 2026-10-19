---
title: audio and midi setup
order: 130
summary: Choose your audio interface, sample rate and buffer size, check your input, and fix common audio problems.
platform: all
audience: beginner
covers:
  - audio-midi-setup/settings-audio-driver-output-input-sample-rate-buffer #the-audio-settings
  - audio-midi-setup/in-the-browser-the-system-picks-the-devices #in-the-browser
  - audio-midi-setup/buffer-size-what-you-ask-for-vs-what-the-device-runs #what-you-ask-for-and-what-the-device-runs
  - audio-midi-setup/smaller-buffer-lower-latency-more-dropouts #latency-and-dropouts
  - audio-midi-setup/recording-latency-is-compensated-automatically #recording-latency
  - audio-midi-setup/input-check #input-check
  - audio-midi-setup/refresh-devices-after-plugging-in #plugging-in-a-new-device
  - audio-midi-setup/unplugged-output-automatic-fallback-and-return #when-the-output-device-disappears
  - audio-midi-setup/device-closed-status-when-another-engine-plays #device-closed
  - audio-midi-setup/engine-not-responding-notice #the-engine-is-not-responding
  - audio-midi-setup/engine-log-desktop #the-engine-log
---

This page is about getting sound in and out: which device plays, which one records, and how to trade latency for stability. Most of it applies to the desktop app. On the web, the browser and your system choose the devices.

## the audio settings [[desktop]]

Click the gear at the top right of the window to open **settings**, then go to the **audio** tab. You can also open it from the command palette ([[action:palette.open]]) with **audio settings…**.

| setting | what it is |
|---|---|
| **driver** | the audio system to use, for example ASIO or WASAPI on Windows. It is only shown when your computer has more than one. |
| **output device** | where you hear Ethereal. The system default is marked **(system default)**. |
| **input device** | what you record from. It is **none (no recording)** until you choose one. Each device shows its channel count. |
| **sample rate** | the rate the device runs at, for example 48000 Hz. |
| **buffer size** | how many samples the device processes at a time (see [latency and dropouts](#latency-and-dropouts)). |

Every change applies at once and is remembered for the next time you start the app. There is no apply button.

### in the browser [[web]]

On the web, the audio tab can't list or change devices. It says that your system chooses the devices. To use other speakers or headphones, change the output in your operating system or your browser. Recording from an audio input needs the desktop app (see [recording](/docs/making-music/recording)).

## latency and dropouts

Under the settings, a status line shows what the device is actually doing: whether it is running, the driver, the sample rate, the buffer size, the output and input latency in milliseconds, and the number of dropouts since it started (when there were any).

The buffer size is a trade-off:

- A **smaller buffer** gives lower latency: what you play or sing comes back sooner. That matters when you record or play an instrument live. But the computer has less time to compute each block, so you get more **dropouts**: clicks and crackles.
- A **larger buffer** is more stable but adds delay. It is fine for mixing and editing.

If you hear crackles, raise the buffer size. If playing live feels late, lower it.

### what you ask for and what the device runs

The buffer select offers sizes from 32 to 2048 samples, limited to what your device supports. It shows the size the device really runs, which is not always the one you asked for:

- If the device runs a different size, a note under the settings says so, for example "the device runs 256 samples (128 asked)".
- With WASAPI shared mode on Windows, the select is disabled: Windows sets the buffer size. Use an ASIO driver to choose it yourself.

### recording latency

When you record, Ethereal places the recorded audio and MIDI using the latency the driver reports (input plus output plus buffering), so takes line up with what you heard. There is no manual offset setting. If your driver reports a wrong latency, recordings can be slightly early or late.

## input check

At the bottom of the audio tab, **input check** shows a level meter for each armed track. Use it to confirm your interface works before you record. It needs three things:

1. an input device chosen above,
2. a track armed (● on its header),
3. that track's monitoring on (see [monitoring](/docs/making-music/recording#monitoring)).

## plugging in a new device

If you plug in an interface or headphones while Ethereal is open and they don't appear in the lists, click the refresh button at the top of the audio tab.

## midi controllers

There is no MIDI device list to set up. MIDI keyboards and controllers connected to your computer are used as they are: a MIDI track listens to **all ports** by default. To make a track listen to one controller or one channel only, see [MIDI input filters](/docs/making-music/recording#midi-input-filters). To control knobs with a controller, see [MIDI learn](/docs/advanced/midi-learn).

On the web, hardware MIDI input isn't available: use the [computer MIDI keyboard](/docs/making-music/computer-midi-keyboard).

## troubleshooting

### when the output device disappears

If your output device goes away while Ethereal plays (a USB interface unplugged, Bluetooth headphones switched off), a warning names the device and Ethereal switches to the default output, so you may suddenly hear the laptop speakers. If even that fails, the engine keeps running silently. When the device comes back, Ethereal switches back to it and tells you it "is back".

Your saved settings don't change: the fallback is temporary. Choosing a device in the audio tab ends it.

### device closed

When this window plays through an engine on another computer (an [engine server](/docs/advanced/engine-server)), the status line says "device closed: no local engine plays". This is expected: your audio device is free while you don't use it. It opens again with the same settings when you switch back.

### the engine is not responding

If the audio engine stalls, for example while a plugin is busy opening its editor, a warning says "the engine is not responding", with the reason when it is known. Audio keeps playing, and what you click is queued and runs when the engine recovers. Don't click the same thing again and again: every click will apply later. When it is over, a message says how long it lasted.

### the engine log [[desktop]]

In **settings › advanced**, **open engine log** opens a terminal window that follows the engine's log as it runs: warnings, plugin problems and why the engine stopped responding. It is useful when you [report an issue](https://github.com/diegoimbert/ethereal/issues).

## where next

- [Recording](/docs/making-music/recording): arm a track and record.
- [Troubleshooting](/docs/reference/troubleshooting): more fixes for audio and plugins.
