---
title: versions and backups
order: 50
summary: Go back to an earlier state of your project with automatic versions and named snapshots, and recover your work after a crash.
platform: all
audience: beginner
covers:
  - projects-storage/versions-automatic-backups #automatic-versions
  - projects-storage/restoring-a-version-is-itself-reversible #restoring-a-version
  - projects-storage/versions-don-t-include-audio-files #what-a-version-contains
  - projects-storage/save-snapshot-mod-alt-s #snapshots
  - projects-storage/crash-recovery-dialog #after-a-crash
---

Besides [undo](/docs/basics/undo-and-history), which only goes back through the edits since you opened the project, Ethereal keeps older **versions** of each project. You can compare them with what you have now and restore one. This page also explains what happens after a crash.

## automatic versions

While you work, Ethereal keeps a version of the project about once a minute, as long as something changed. You don't have to do anything.

To keep the list short, older versions are thinned out: at most 30 automatic versions are kept, spread from the last few minutes back to the very first one. Versions you named (snapshots) are never removed.

Open the list from the project menu (☰) › **versions…**, or with **versions…** in the command palette. Newest versions come first. For each one you can:

- **compare** it with the project as it is now: the dialog lists what was added, removed or changed since;
- **restore** it;
- from its **⋯** menu, **rename** it (which also keeps it for good) or **delete…** it.

The field at the top of the dialog saves a version of the project as it is now, with an optional name.

## restoring a version

**Restore** replaces the project with that version. Before it does, Ethereal keeps the current state as a version marked **before restore**. So a restore is never a dead end: if you change your mind, restore the **before restore** version.

Only the 10 newest unnamed **before restore** versions are kept.

## what a version contains

A version holds the project itself: the tracks, clips, notes, devices, automation and settings. It does **not** hold your audio files, sharing information or exported files.

> [!warning]
> If you delete or replace an audio file, restoring an older version won't bring it back. The clips that used it show as missing.

## snapshots

A snapshot is a version with a name, such as "before the mixdown", that is kept until you delete it. Make one with:

- the project menu › **save snapshot…**;
- [[action:project.snapshot]];
- **save snapshot…** in the command palette.

Type a name and press **save snapshot**. It shows up in **versions…** with its name.

## after a crash

If Ethereal didn't close properly last time (a crash, a power cut), the next launch shows a dialog before anything else.

If a project has changes newer than its last save, the dialog is **recover unsaved work?**, with three choices for each project:

- **recover** opens that newer, unsaved work. It isn't saved yet: make sure it's what you want, then save ([[action:project.save]]) to keep it.
- **keep saved** forgets the unsaved work and keeps the last saved state.
- **open without plugins** opens the saved project with its plugins turned off, in case a plugin caused the crash (see [safe mode](/docs/advanced/safe-mode)).

If there was no unsaved work (for example, a plugin crashed while the project was loading), the dialog is **Ethereal didn't close properly**, with **open**, **open without plugins** and **dismiss**.

**Decide later** closes the dialog without choosing; it asks again at the next launch.

## where next

- [Undo and history](/docs/basics/undo-and-history)
- [Moving projects between computers](/docs/basics/moving-projects)
- [Safe mode](/docs/advanced/safe-mode)
