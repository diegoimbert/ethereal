---
title: projects and saving
order: 40
summary: Create, open, rename and delete projects, start from templates, and see how Ethereal saves your work for you.
platform: all
audience: beginner
covers:
  - first-run/the-welcome-screen-is-the-project-picker #the-welcome-screen
  - first-run/closing-the-welcome-screen-drops-you-into-a-draft-project #new-projects-start-as-drafts
  - first-run/untouched-draft-projects-are-thrown-away #new-projects-start-as-drafts
  - first-run/get-started-guided-tours-and-soon-items #guided-tours
  - first-run/reopen-last-project-on-launch-off-by-default #reopening-your-last-project
  - first-run/project-links-in-the-address-bar-web #project-links-in-the-address-bar
  - first-run/project-templates-and-the-default-template #templates
  - projects-storage/the-project-menu #the-project-menu
  - projects-storage/some-project-actions-are-only-in-the-command-palette #actions-in-the-command-palette
  - projects-storage/recent-projects-search-right-click-menu #recent-projects
  - projects-storage/autosave-and-the-unsaved-changes-dot #saving-is-automatic
  - projects-storage/plugin-tweaks-don-t-show-the-unsaved-dot #plugin-settings-and-the-dot
  - projects-storage/closing-a-web-tab-doesn-t-warn-you #closing-the-browser-tab
  - projects-storage/quitting-the-desktop-app #quitting-the-desktop-app
  - projects-storage/cmd-w-closes-the-top-floating-window-not-the-app #quitting-the-desktop-app
  - projects-storage/deleting-a-project-is-permanent #deleting-a-project
  - projects-storage/files-from-newer-or-pre-release-versions-are-refused #projects-from-a-newer-version
---

A project is one song: its tracks, clips, devices and settings. This page explains how to make and find projects, and how they're saved. To know where projects are stored (and why that matters on the web), read [where your projects live](/docs/welcome/where-your-projects-live) first.

## the welcome screen

When Ethereal starts, it shows the welcome screen. It's also your list of projects (the **open project** screen). It has:

- the Ethereal logo and the version you're running;
- **new project**;
- a **get started** list of guided tours;
- your **recent projects**.

Nothing opens by itself unless you ask for it (see [reopening your last project](#reopening-your-last-project)). You can open the same screen later from the project menu: **open project…**.

### new projects start as drafts

If you close the welcome screen ([[key:Escape]] or a click outside it) without picking a project, you land in a **new project** named after today's date. **New project** in the project menu does the same.

A new project starts from your [default template](#templates). If you haven't set one, you get the starter project: one MIDI track called **synth**, with the built-in Synth on it, ready for you to draw notes.

A new project is a **draft** until you change something in it (an edit, an undo step or a rename). If you leave a draft untouched, by opening another project or quitting, it's deleted and never shows up in your recent projects. So feel free to close the welcome screen just to look around.

### guided tours

The **get started** list on the welcome screen starts guided tours of the real app, such as making your first track or recording audio. Starting one makes a project for it and walks you through the steps. When you finish or leave the tour, you come back to the welcome screen.

A tour you've finished shows a check mark. Clicking it again asks whether you want to **do it again**. On a phone the guides are marked **soon**: the tours run in the desktop app and the web version.

### reopening your last project

In **settings › general**, under **startup**, **reopen last project on launch** opens the project you were working on when Ethereal starts. It's off by default.

It only reopens the project if Ethereal closed properly last time. After a crash you choose what to do in the [recovery dialog](/docs/basics/versions-and-backups#after-a-crash). With the setting off, Ethereal loads no plugins until you pick a project, which is handy if a plugin is causing trouble.

## recent projects

The welcome screen lists your projects, newest first. The project that's open right now isn't in the list.

- **Type** in the search box to filter the list by name. [[key:Enter]] opens the first match.
- **Click** a project to open it.
- **Right-click** a project, or use its **⋯** button, for more: **open**, **open without plugins** (see [safe mode](/docs/advanced/safe-mode)), **rename**, **duplicate**, **export…** and **delete…**.

## the project menu

The **☰** button at the top left opens the project menu:

- **new project** and **open project…**;
- **save as template…**;
- **project scale** (see [the project scale](/docs/basics/tempo-and-time-signature#the-project-scale));
- **import project…**;
- **export project…** and **export audio…**;
- **versions…** and **save snapshot…** (see [versions and backups](/docs/basics/versions-and-backups));
- **resolve missing devices and samples…**, only while something the project uses is missing (see [missing plugins and samples](/docs/sounds-and-plugins/missing-plugins-and-samples));
- **delete project…**;
- **news…**.

To rename the project, click its name next to the ☰ button, type the new name and press [[key:Enter]].

### actions in the command palette

A few project actions are only in the [command palette](/docs/basics/command-palette-and-settings) ([[action:palette.open]]):

- **save as…**: saves a copy of the project, media included, under a new name, and switches to the copy;
- **duplicate project**: makes a copy and stays where you are;
- **rename project**;
- **new project from template…**;
- **relink missing samples…** and **collect all and save** (see [moving projects](/docs/basics/moving-projects#missing-samples)).

Duplicate and rename are also in a recent project's ⋯ menu.

## templates

A template is a starting point for new projects.

- **Save as template…** in the project menu saves the whole project (tracks, devices, plugin states and settings) as a project template. Its samples are copied into your library, so the template still works after you delete the project.
- **New project from template…** in the command palette lets you name a new project and pick what it starts from: **empty project** or one of your templates.
- In that list, right-click a template and choose **use for new projects** to make it your **default**. From then on every new project, including the one you land in when you close the welcome screen, starts from it. **Stop using as default** goes back to the starter project.

## saving is automatic

There's no save step to remember. Ethereal saves the project about a second after your last edit. While there are changes that aren't saved yet, a **dot** shows after the project's name in the top bar.

[[action:project.save]] saves at once. It works from anywhere, even while you're typing in a text field.

### plugin settings and the dot

Turning a knob inside a plugin's own window doesn't show the dot, because the app can't see inside the plugin. Ethereal still saves plugin settings in the background, a little more slowly. [[action:project.save]] reads every plugin's current settings right away, and quitting the desktop app saves them too.

### closing the browser tab [[web]]

In the browser, closing the tab gives no "leave this page?" warning. When you hide or close the tab, Ethereal saves any change at once, but a browser can only wait a moment for that. To be safe, wait until the dot after the project's name is gone before you close the tab.

### quitting the desktop app [[desktop]]

Quitting ([[key:Mod+Q]], or closing the main window) asks **quit Ethereal?** and tells you that your changes are saved before it closes. [[key:Enter]] confirms. Plugin settings are saved too; this can take a few seconds with many plugins (at most 30 seconds). If the project still can't be saved, a copy is kept in the app's **recovered** folder, and the next launch tells you where it is.

[[key:Mod+W]] does not quit: it closes the topmost floating window, such as a device or plugin window.

## deleting a project

**Delete project…** (in the project menu, or a recent project's menu) asks once, then deletes the project with all its [versions](/docs/basics/versions-and-backups).

> [!warning]
> Deleting a project is permanent. There is no trash and no undo. If you might want it back one day, [export it](/docs/basics/moving-projects#exporting-a-project) first.

If you delete the project that's open, Ethereal first switches to your most recent other project, or to a new draft if there's none.

## project links in the address bar [[web]]

In the browser, the address of an open project looks like `/p/my-song/<id>`. Reloading the page reopens that project, and the browser's back and forward buttons switch between the projects you opened.

The link only works in the browser that holds the project. Opened anywhere else, it shows the project list with **this project isn't saved in this browser**. To take a song to another browser or computer, [export it](/docs/basics/moving-projects) instead.

## projects from a newer version

A project saved by a newer version of Ethereal won't open in an older one: you'll see **this file was made with a newer version of Ethereal**, and you need to update. Projects saved by pre-release builds of Ethereal (before the first public release) can't be opened at all.

## where next

- [Versions and backups](/docs/basics/versions-and-backups)
- [Moving projects between computers](/docs/basics/moving-projects)
- [Undo and history](/docs/basics/undo-and-history)
