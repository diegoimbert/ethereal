---
title: where your projects live
order: 20
summary: Where the desktop app and the web version keep your projects, and how not to lose them.
platform: all
audience: beginner
covers:
  - projects-storage/where-your-projects-are-stored #on-the-desktop
  - projects-storage/web-projects-can-be-lost-with-browser-data #in-the-browser
  - remote-engine/web-projects-live-in-the-browser #in-the-browser
  - first-run/only-one-browser-tab-can-use-ethereal #one-tab-at-a-time
---

Ethereal saves your work as you go: there's no "save as" file to pick. Where it keeps your projects depends on how you run it, and the two places are separate. Read this once before you start a project you care about.

## on the desktop [[desktop]]

The desktop app keeps each project as a folder in **Documents › Ethereal › Projects** in your home folder. It's a normal folder on your disk: your usual backups (Time Machine, File History, a synced folder) include it.

Your recordings are saved inside the project's folder. Samples you drag in from your library or a folder on your disk stay where they are, and the project points to them (see [moving projects](/docs/basics/moving-projects#samples-from-your-disk)).

## in the browser [[web]]

The web version keeps your projects in this browser's private storage for the site, on this computer only. They aren't files you can see on your disk, the desktop app can't see them, and another browser (or another computer) has its own, separate, list.

> [!warning]
> Web projects live only in this browser. Clearing the site's data or cookies, a private window that closes, or the browser freeing space when your disk is full can delete them. Export the projects you care about now and then (project menu › **export project…**), or use the desktop app.

To make this less likely, Ethereal asks the browser to keep its storage when it starts. Some browsers agree at once, some decide by how much you use the site (a bookmark or an installed app helps), and Firefox asks you. If the browser says no, Ethereal tells you once. Settings › general › **storage** shows where you stand:

- **persistent**: the browser keeps your projects.
- **may be cleared by the browser when space runs low**: with **request again** to ask once more.
- **this browser can't keep storage permanently**.

It also shows how much space your projects use.

## moving a project

To take a project from the browser to the desktop app, or to another computer, export it as an `.ether` file and import it there. See [moving projects between computers](/docs/basics/moving-projects).

## one tab at a time [[web]]

Only one browser tab can work on your projects at a time. If you open Ethereal in a second tab, it shows **ethereal is open in another tab** with **use here**. **Use here** moves Ethereal to the new tab: the first tab saves your work, stops, and shows **ethereal moved to another tab** (with its own **use here** to bring it back). If the first tab doesn't answer within a few seconds, because it froze, the new tab takes over anyway.

The desktop app has no such limit, and these docs can be open in as many tabs as you like.

## where next

- [Your first loop](/docs/welcome/first-beat): make some music in five minutes.
- [Projects and saving](/docs/basics/projects): the welcome screen, the project menu, autosave.
- [Versions and backups](/docs/basics/versions-and-backups): go back to an earlier state of a project.
