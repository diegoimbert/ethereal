---
title: exporting audio
order: 30
summary: Render your song, a loop or a selection to WAV, FLAC or MP3, as one mix or as stems.
platform: all
audience: beginner
covers:
  - export/export-audio-dialog #the-export-audio-dialog
  - export/where-exported-files-go #where-the-files-go
  - export/range-options-and-the-selected-clips-range #range
  - export/tail-for-reverb-and-delay #tail
  - export/formats-bit-depth-and-sample-rate-rules #format-bit-depth-and-sample-rate
  - export/normalize #normalize
  - export/stems-what-each-file-contains #stems
  - export/overlapping-stems #when-stems-overlap
  - export/export-keeps-running-when-the-dialog-is-closed #while-it-exports
  - export/large-exports-in-the-browser #large-exports-in-the-browser
  - export/a-plugin-that-can-t-render-offline-fails-the-export #when-a-plugin-stops-the-export
---

When your song is ready to share, export it as an audio file. This page goes through every option of the export dialog, including stems (one file per track).

## the export audio dialog

Open the project menu (top left) and choose **export audio…**. The defaults are ready to go: the whole project as a 24-bit WAV at the engine's sample rate. Click **export**.

The options:

| Option | Choices | Default |
|---|---|---|
| **range** | whole project, loop region, selected clips | whole project |
| **format** | WAV, FLAC, MP3 (320 kbps) | WAV |
| **bit depth** | 16-bit (dithered), 24-bit, 32-bit float | 24-bit |
| **sample rate** | engine rate, 44.1, 48, 88.2 or 96 kHz | engine rate |
| **tail** | 0 to 60 seconds | 0 |
| **file name** | any name | the project's name |
| **normalize** | on or off | off |
| **stems** | on or off | off |

### range

- **whole project**: from the start to the end of the last clip (or the last automation point on a track, if that comes later).
- **loop region**: just the part between the loop brace's ends.
- **selected clips**: the time selection, if you made one ([[key:Shift]]-drag in the arrangement), otherwise the span from the first selected clip to the end of the last one. It's greyed out until something is selected.

### tail

The export stops exactly at the end of the range, which cuts off reverb and delay that are still ringing. Set **tail** to a few seconds to let them die out.

### format, bit depth and sample rate

- **WAV** takes any bit depth. 16-bit is dithered.
- **FLAC** is lossless and smaller than WAV, in 16 or 24 bits only.
- **MP3** is always 320 kbps and has no bit depth. It can only be 44.1 or 48 kHz (or the engine rate).

Choices that don't fit the format are greyed out. **engine rate** exports at the sample rate your audio device runs at.

### normalize

**normalize** raises or lowers the whole file so its loudest peak sits at -0.1 dBFS. It's peak normalization, not loudness matching: it doesn't make quiet songs sound as loud as commercial masters. With stems, each stem is normalized on its own.

## stems

Turn on **stems** to export one file per track instead of one mix. Pick the tracks in the list: audio and MIDI tracks are picked by default. Each stem contains:

- the track's own sound through its devices, volume and pan,
- plus what it sends to return tracks.

Solo is ignored, and a muted track is unmuted for its own stem. A track inside a group skips the group's devices. A group's stem includes its tracks. The master track's devices are left out of every stem.

### when stems overlap

If you pick a track together with its group, or with a return it sends to, that audio ends up in both files. A sidechain that listens to another track hears silence in a stem. So the stems only add up exactly to the mix when the master and groups have no processing that changes the sound, and there are no sidechains.

## while it exports

A progress bar shows how far the render is. **cancel export** stops it. You can close the dialog and keep working: the top bar shows **export n%**, and clicking it reopens the dialog.

## where the files go

- **Desktop** [[desktop]]: the files are saved in the `exports` folder inside the project's folder, and the dialog lists them when it's done. An existing file is never overwritten: a new one gets a number, like "my song (2).wav".
- **Web** [[web]]: each file downloads through your browser. Until you close the dialog, a **download again** button sits next to each file.

### large exports in the browser

In the browser, the exported files are kept in memory until they're downloaded. Above about 200 MB, the dialog warns you. Long stems exports at high sample rates can get that big. Normalizing also holds the whole render in memory, on every platform.

### when a plugin stops the export

The export renders offline, with fresh copies of your plugins loaded with their current settings. If a plugin can't be loaded that way, the export stops with an error instead of quietly leaving the plugin out.

## where next

- [Mixing](/docs/making-music/mixing): get the balance right first.
- [Moving projects between computers](/docs/basics/moving-projects): send the whole project as one .ether file.
