---
title: the computer midi keyboard
order: 40
summary: Play and record notes with your computer keyboard, with no MIDI hardware.
platform: all
audience: beginner
covers:
  - midi-editing/computer-midi-keyboard-m #turning-it-on
  - midi-editing/while-the-midi-keyboard-is-on-letter-shortcuts-stop-working #shortcuts-while-it-is-on
  - midi-editing/the-midi-keyboard-auto-arms-a-track #which-track-it-plays
  - midi-editing/the-midi-keyboard-keeps-playing-inside-plugin-windows #inside-plugin-windows
---

You don't need a MIDI controller to play notes. The computer MIDI keyboard turns your computer's letter keys into a small piano. It works on the desktop app and on the web, where it is the only way to play notes live.

You need a MIDI track with an instrument. See [tracks](/docs/making-music/tracks) and [instruments](/docs/making-music/instruments).

## turning it on

Press [[action:midi.computerKeyboard]], or click the piano button in the transport bar. While it is on, the button is lit and shows the current octave and velocity next to it. Press the key again to turn it off.

It stays on or off between sessions: if you leave it on, it is still on the next time you open Ethereal.

## the keys

The layout is the same as Ableton Live's. It uses key positions, not letters, so it works the same on QWERTY, AZERTY and QWERTZ keyboards.

| keys | what they do |
|---|---|
| the home row, **A S D F G H J K L ;** | white keys, starting from C |
| the row above, **W E T Y U O P** | black keys |
| **Z** / **X** | one octave down / up |
| **C** / **V** | velocity down / up, in steps of 20 |

It starts at octave 3 (C3 is MIDI note 60) with a velocity of 100. The octave and velocity are remembered too.

## which track it plays

The keyboard behaves exactly like a hardware MIDI input. Every MIDI track that is monitoring live input hears it, and every armed MIDI track records it. [Capture MIDI](/docs/making-music/capture-midi) keeps what you play too.

If no MIDI track would hear it, turning the keyboard on (or playing a note) arms the selected MIDI track for you, or the first MIDI track if none is selected. Ableton does the same. Hover the piano button to see which track it plays on. If the project has no MIDI track, the tooltip tells you to add one.

> [!note]
> Because of this auto-arm, a track can be armed without you clicking its record button. The next time you record, check which tracks are armed.

## shortcuts while it is on

While the keyboard is on, its keys (the note keys and Z, X, C, V) play notes instead of running any shortcut bound to them. Shortcuts with [[key:Mod]], [[key:Ctrl]], [[key:Alt]] or [[key:Shift]] still work, and so does [[action:transport.play]]. Typing in a text field is never affected.

With the default shortcuts, none of the single-letter shortcuts (such as [[action:pianoRoll.drawMode]] for draw mode or [[action:clip.reverse]] for reverse) use these keys. If you bind your own shortcuts to them, they won't work while the keyboard is on.

### inside plugin windows [[desktop]]

On the desktop app, a plugin's own editor window passes these keys on to Ethereal while the keyboard is on. You can keep playing a synth while you tweak it in its window. When the keyboard is off, the plugin window gets your typing as usual.

## where next

- [The piano roll](/docs/making-music/piano-roll): edit the notes you recorded.
- [Recording](/docs/making-music/recording): record what you play.
- [Capture MIDI](/docs/making-music/capture-midi): keep something you played without recording.
