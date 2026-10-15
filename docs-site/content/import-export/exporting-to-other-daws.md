---
title: exporting to other daws
order: 40
summary: Save your project as a DAWproject, Ableton Live set or FL Studio project, and know what won't make the trip.
platform: all
audience: everyone
covers:
  - export/export-to-another-daw #exporting-a-project
  - export/als-and-flp-exports-are-untested-in-live-fl #which-format-to-choose
  - export/what-other-daws-can-t-receive #what-other-daws-cant-receive
  - export/fl-studio-may-ask-to-locate-samples #fl-studio-and-its-samples
---

Ethereal can write your project as a project file of another DAW, so you can finish a song elsewhere or hand it to someone who doesn't use Ethereal. This page covers the formats, which one to pick, and what gets lost on the way. To share just the sound, [export audio](/docs/import-export/exporting-audio) instead.

## exporting a project

Open the project menu (top left) and choose **export to another daw**, then one of:

- **DAWproject (.dawproject)**: opens in Bitwig Studio, Studio One, Cubase, Nuendo, Cubasis, VST Live, n-Track Studio and Waveform.
- **Ableton Live Set (.als)**: a Live 11 set.
- **FL Studio Project (.flp)**: an FL Studio 20 project.

The same choices are in the command palette as **export to …**.

- **Desktop** [[desktop]]: Ethereal asks where to save the file. For Live and FL Studio, the audio files are written into a `Samples` folder next to it.
- **Web** [[web]]: the file downloads. Live sets and FL Studio projects download as a `.zip` that holds the project and its `Samples` folder: unzip it before opening.

When it's done, a report shows what was exported, then what was **exported approximately** and what was **not exported**.

## which format to choose

> [!warning]
> The Ableton Live and FL Studio exports haven't been tested in Live or FL Studio yet. Ethereal checks them by importing them back, but a file may still fail to open. DAWproject is the most reliable route.

If your DAW reads DAWproject, use it. Ableton Live and FL Studio can't read DAWproject, so for them use their own formats.

## what other daws can't receive

Some things exist only in Ethereal, so they're left out or simplified. The report lists each one.

- **Ethereal's built-in devices** (delay, reverb, sampler, racks…) aren't exported to Live or FL Studio. In DAWproject, only the EQ, compressor, gate and limiter survive, as the format's generic devices.
- **Sidechains and modulators** are dropped in every format.
- **Reversed audio** plays forwards in DAWproject and Live.
- **Take lanes** don't reach DAWproject: only the clips that play are exported. Live gets them.
- **FL Studio** gets VST2 plugins only (no VST3, CLAP or AU), and no automation, tempo changes or fades.
- **CLAP and AU plugins** aren't exported to Live.

## fl studio and its samples

The FL Studio export copies your audio into a `Samples` folder next to the `.flp`. FL Studio may still ask you where the files are: point it to that folder. Keep the `Samples` folder beside the project whenever you move it.

## where next

- [Projects from other DAWs](/docs/import-export/other-daws): open their projects in Ethereal.
- [Exporting audio](/docs/import-export/exporting-audio): a mix or stems.
- [Moving projects](/docs/basics/moving-projects): take an Ethereal project to another computer.
