---
title: moving projects between computers
order: 60
summary: Export a project as one .ether file, open it on another computer or between the web and desktop versions, and fix samples that went missing.
platform: all
audience: beginner
covers:
  - projects-storage/moving-a-project-between-web-and-desktop-or-to-another-compu #moving-a-project
  - projects-storage/export-project-pack-media-files-or-not #exporting-a-project
  - projects-storage/missing-samples-and-relinking #missing-samples
  - projects-storage/samples-used-from-your-disk-aren-t-inside-the-project #samples-from-your-disk
---

Your projects are stored on the computer, or in the browser, where you made them (see [where your projects live](/docs/welcome/where-your-projects-live)). To work on a song somewhere else, between the web and the desktop app, or just to keep a backup, you export it as a **.ether** file and import that file on the other side.

To send a song to someone so they can listen to it or edit it with you, [sharing](/docs/sharing/overview) is usually easier.

## moving a project

1. On the first computer, open the project and choose **export project…** in the project menu (☰). See [exporting a project](#exporting-a-project) below.
2. Move the .ether file to the other computer (a USB stick, a cloud drive, email…).
3. On the other computer, choose **import project…** in the project menu, or simply drop the .ether file onto the Ethereal window.

The import is added as a new project and opens. If you already have a project with the same name, the new one gets a number, like "Song 2". Nothing you already have is replaced.

This works in every direction: desktop to desktop, web to desktop, desktop to web.

## exporting a project

**Export project…** (in the project menu, or in a recent project's menu on the welcome screen) opens a small dialog with one choice, **pack media files**:

- **On** (the default): one self-contained .ether file with the project and every audio file it uses, including samples from your library or elsewhere on your disk. Use this to move a project or to keep a backup.
- **Off**: only the project itself, which is much smaller. The audio files aren't included, so on another computer the samples show as missing.

Press **export**. The desktop app asks where to save the file; the browser downloads it.

> [!tip]
> Exporting with **pack media files** on is also the best way to keep a backup of a web project outside the browser.

## samples from your disk

When you drag a sample into a project from your library or from a folder on your disk, Ethereal doesn't copy it into the project: the project points to the file where it is. If you later move, rename or delete that file, the sample goes missing.

To make a project independent of those files, use **collect all and save** in the command palette (or in the missing samples dialog). It copies every sample the project uses from outside into the project's own folder, then saves. It's a single undo step.

## missing samples

When a project can't find some of its audio files, a notice says **1 sample is missing** (or **N samples are missing**) with a **relink…** button. The clips that use a missing file play silence until you find it.

**Relink…** opens the **missing samples** dialog. It lists each missing file with a **missing** or **found** badge, and gives you several ways to find them:

- **locate…** on a row: pick the file yourself;
- **search library**: look through your library for the missing files;
- **search in folder…** [[desktop]]: look through a folder you choose.

A search looks for files with the same name. A file with exactly the same content is relinked at once. A file with the same name but different content is listed under its sample with a **use** button, so you decide.

The dialog also has **collect all and save** (see [above](#samples-from-your-disk)).

You can also open the dialog with **relink missing samples…** in the command palette, or right-click a clip whose sample is missing and choose **relink sample…**.

For missing plugins and devices too (for example after importing a project from another DAW), see [projects from other DAWs](/docs/import-export/other-daws).

## where next

- [Versions and backups](/docs/basics/versions-and-backups)
- [Where your projects live](/docs/welcome/where-your-projects-live)
- [Sharing overview](/docs/sharing/overview)
