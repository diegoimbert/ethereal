---
title: your first loop
order: 30
summary: Make a short loop in five minutes: notes, a second instrument, an effect, and playback.
platform: all
audience: beginner
covers:
---

This walk-through takes you from an empty project to a short loop of two instruments with an effect. It uses only what comes with Ethereal, in the desktop app or the web version. Every step links to the page that explains it in more detail.

## 1. start a project

When Ethereal starts, it shows the welcome screen. Click **new project**, or close the welcome screen with [[key:Escape]]: either way you get a new project with one MIDI track called **synth**, with the built-in synth on it.

You don't need to save: Ethereal saves as you go (see [projects and saving](/docs/basics/projects)).

## 2. make a clip

A clip holds notes. Double-click the empty lane of the **synth** track, near the start: a one-bar clip appears. Click the clip to open it in the piano roll, at the bottom of the window.

## 3. draw some notes

In the piano roll, rows are pitches (the keyboard on the left) and columns are time.

- **Double-click** an empty spot to add a note. Keep the button down after the second click and drag right to make it longer.
- Drag a note to move it, drag its right edge to change its length, and double-click it to delete it.
- Press [[action:pianoRoll.drawMode]] for draw mode, where every click adds a note. Press it again to go back.

Draw four or five notes. You hear each note as you add it. More in [the piano roll](/docs/making-music/piano-roll).

> [!tip]
> No ideas? Turn on the [computer MIDI keyboard](/docs/making-music/computer-midi-keyboard) with [[action:midi.computerKeyboard]] and play notes on your letter keys.

## 4. play it

Press [[action:transport.play]] to play, and again to stop and go back to where you started. To hear the bar over and over, turn on the **loop** button in the top bar: playback repeats the region of the loop brace above the tracks, which you can drag to cover your clip. See [playing and moving around](/docs/basics/playing-and-navigating).

## 5. add a second instrument

Click **add track** above the track headers and choose **midi track**. The instrument library opens under the new track: pick another instrument, for example the **poly synth**, or click away to keep the default synth. Make a clip on this track too and draw a bass line or a few chords, lower down. See [tracks](/docs/making-music/tracks) and [instruments](/docs/making-music/instruments).

Each instrument has factory presets: select the track, then in the **inspector** on the right hover the instrument's card and open **presets**. Try a few.

## 6. add an effect

With a track selected, the inspector shows its devices. Click **add audio effect** under the instrument and choose **reverb**. Its window opens: turn a knob and listen. See [effects](/docs/making-music/effects).

## 7. balance it

Each track header has a volume fader and a pan knob. Turn down whatever is too loud. If you make a mistake anywhere, [[action:edit.undo]] undoes it.

## where next

You made a loop. From here:

- [A tour of the screen](/docs/basics/the-screen): what every part of the window is for.
- [Clips](/docs/making-music/clips): copy, loop and arrange your clips into a song.
- [The browser](/docs/sounds-and-plugins/browser): bring in your own samples, and build a beat with the [drum rack](/docs/making-music/drum-rack).
- [Exporting audio](/docs/import-export/exporting-audio): turn your song into a WAV or MP3 file to share.
