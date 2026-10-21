---
title: keyboard shortcuts
order: 10
summary: Every default shortcut, generated from the app's own keymap, and how to change them.
platform: all
audience: everyone
covers:
  - shortcuts-commands/transport-shortcuts #all-shortcuts
  - shortcuts-commands/edit-shortcuts #all-shortcuts
  - shortcuts-commands/time-selection-shortcuts #all-shortcuts
  - shortcuts-commands/arrangement-shortcuts #all-shortcuts
  - shortcuts-commands/piano-roll-shortcuts #all-shortcuts
  - shortcuts-commands/automation-lane-shortcuts #all-shortcuts
  - shortcuts-commands/app-and-view-shortcuts #all-shortcuts
  - shortcuts-commands/copy-cut-paste-act-on-the-time-selection-when-there-is-one #the-time-selection-comes-first
  - shortcuts-commands/computer-midi-keyboard-m #the-computer-midi-keyboard
  - shortcuts-commands/mouse-modifiers #all-shortcuts
  - shortcuts-commands/customizing-shortcuts #changing-shortcuts
  - shortcuts-commands/ableton-like-preset #the-ableton-like-preset
  - shortcuts-commands/conflicts #conflicts
  - shortcuts-commands/any-palette-command-can-get-a-shortcut #any-command-can-get-a-shortcut
  - shortcuts-commands/print-the-cheat-sheet #printing-a-cheat-sheet
  - shortcuts-commands/where-shortcuts-are-saved #where-your-shortcuts-are-saved
  - shortcuts-commands/ctrl-vs-on-mac #mac-and-pc-keys
  - shortcuts-commands/browser-shortcuts-that-win-on-the-web #in-the-browser
  - shortcuts-commands/shortcuts-in-text-fields-and-plugin-windows #text-fields-and-plugin-windows
---

This page lists every default shortcut. It is generated from the app's own keymap, so it is always up to date. Use the **mac** / **windows / linux** switch at the top of the page to see the keys for your computer. Most actions are also in the command palette ([[action:palette.open]]), with their shortcut next to them.

## all shortcuts

"Where" says which part of the app needs the focus (click in it first): the arrangement, the piano roll or an automation lane. The others work everywhere, except while you type in a text field.

[[shortcuts]]

## the time selection comes first

In the arrangement, when there is a time selection ([[key:Shift]]-drag across the lanes), the edit keys act on that stretch of time instead of on clips: copy, cut, duplicate and delete take the selected time on the selected tracks, paste puts a copy right after the selection (so pasting again tiles it), and split cuts the clips at both edges. See [arranging](/docs/making-music/arranging).

## the computer midi keyboard

[[action:midi.computerKeyboard]] turns your computer keyboard into a piano: the middle row plays the white keys, the row above the black keys, [[key:Z]] / [[key:X]] change the octave and [[key:C]] / [[key:V]] the velocity. While it's on, those letters play notes instead of their shortcuts. Shortcuts with [[key:Mod]], [[key:Ctrl]] or [[key:Alt]], and [[action:transport.play]], still work. See [the computer midi keyboard](/docs/making-music/computer-midi-keyboard).

## changing shortcuts

Open the command palette and choose **keyboard shortcuts…**, or click **edit keyboard shortcuts…** in **settings › mouse & keyboard › keyboard shortcuts**. The editor lists every action by group; search by name or by key.

- **+** on a row records a new shortcut: press the keys, then **use** (or [[key:Enter]]). [[key:Escape]] cancels. An action can have up to 4 shortcuts.
- **×** next to a shortcut removes it.
- The reset arrow on a row brings back its default.
- **reset all to ethereal defaults** undoes every change.

### the ableton-like preset

The **preset** menu at the top of the editor switches between **ethereal** (the default) and **ableton-like**. The Ableton-like preset adds Ableton Live's keys for the same commands on top of the defaults, and removes nothing: for example [[key:F9]] to record, [[key:Mod+L]] for the loop, [[key:Mod+T]] and [[key:Mod+Shift+T]] for new audio and MIDI tracks. The table above shows them under each action.

### conflicts

When two actions that work in the same place share a key, both rows are flagged. **Conflicts only** shows just those. When you record a key that another action already uses, the editor names it, and you can **use anyway** or take the key away from the other action.

### any command can get a shortcut

Every command in the palette (adding a device, opening a panel…) is in the editor and can get a shortcut, except the ones tied to the current project, like "go to track".

### printing a cheat sheet

**Print keyboard shortcuts** in the command palette prints every shortcut you have now, with the mouse modifiers.

### where your shortcuts are saved

Your shortcuts are saved in your user library, so they're the same in every project and every window. If the engine can't store them, the editor says that changes last until you close Ethereal.

## mac and pc keys

On a Mac, shortcuts use ⌘ (Command). A Control chord that no action uses works as its ⌘ version. On Windows and Linux, the same shortcuts use Ctrl.

## in the browser [[web]]

Some keys belong to the browser and never reach the page: new tab, reopen tab, close tab and new window ([[key:Mod+T]], [[key:Mod+Shift+T]], [[key:Mod+W]], [[key:Mod+N]]). So the Ableton-like keys for new tracks don't work in the web version. Other keys the app uses, such as save, duplicate or the palette, work inside the page.

## text fields and plugin windows

Shortcuts are ignored while you type in a text field ([[action:transport.play]] types a space there). Plugin windows on the desktop pass [[action:transport.play]] on to Ethereal and, while the computer MIDI keyboard is on, its keys, so you can play from inside a plugin.

## where next

- [The command palette and settings](/docs/basics/command-palette-and-settings).
- [Arranging: selections and time edits](/docs/making-music/arranging).
- [The piano roll](/docs/making-music/piano-roll).
