---
title: undo and history
order: 70
summary: Undo and redo any edit, jump back to any step with the history panel, and name checkpoints.
platform: all
audience: beginner
covers:
  - app-shell/undo-and-redo #undo-and-redo
  - app-shell/history-panel-jump-to-any-step #the-history-panel
  - app-shell/checkpoints-in-the-undo-history #checkpoints
  - app-shell/undo-history-isn-t-saved #the-history-isnt-saved
  - app-shell/undo-in-a-jam-only-undoes-your-own-edits #undo-in-a-live-session
---

Every change you make to your song can be undone. This page explains what undo covers, how to jump back several steps at once, and its limits. To go back further than this session, see [versions and backups](/docs/basics/versions-and-backups).

## undo and redo

- [[action:edit.undo]] undoes the last edit.
- [[action:edit.redo]] redoes it.

The **undo** and **redo** buttons in the top bar do the same. Hover them to see which step they would undo or redo, for example "undo move 3 clips".

Every edit to the song can be undone: notes, clips, tracks, devices and their settings, automation, the loop region, the tempo and time signature, mute and solo. A drag or a knob turn is a single step, however long it lasts.

Some things are not edits to the song, so they aren't in the undo history: arming a track for recording, playing, stopping and moving the playhead, opening plugin windows, the audio settings, and project actions such as opening, deleting or duplicating a project.

## the history panel

The **history** icon in the left rail opens the history panel. It lists your edits by name, oldest at the top:

- **Click** a step to undo or redo everything up to that point. Steps after it are shown as undone until you make a new edit.
- **Right-click** a step for **undo to here** or **redo to here**.
- The first row, **project opened**, undoes everything since you opened the project.

The footer counts the steps and how many are undone.

### checkpoints

To find an important moment again easily, give it a name: click the **flag** on a step, or right-click it and choose **name checkpoint…**. The name shows in the list next to the step. **Name checkpoint…** in the command palette names the current step from anywhere.

Right-click a checkpoint to **rename** or **remove** it.

## the history isn't saved

The undo history lasts only while the project is open. Reloading the page, opening another project or quitting clears it, checkpoints included. The panel reminds you: **not saved with the project**.

Only the newest 500 steps are kept. Past that, the oldest are dropped and the panel says **older steps were dropped from the history**; its first row then reads **oldest kept state**.

To keep a state of your song for longer, save a [snapshot](/docs/basics/versions-and-backups#snapshots).

## undo in a live session

In a [live session](/docs/sharing/live-sessions), undo and the history panel only cover **your own** edits. Undoing goes back on what you did and keeps what the others changed since. The panel says so at the top while you're in a session.

## where next

- [Versions and backups](/docs/basics/versions-and-backups)
- [The command palette and settings](/docs/basics/command-palette-and-settings)
- [Live sessions](/docs/sharing/live-sessions)
