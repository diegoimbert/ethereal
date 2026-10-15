---
title: static share links
order: 50
summary: Publish a frozen, playable snapshot of your project as a link that works while you're offline, and control what people can take.
platform: all
audience: everyone
covers:
  - sharing/static-snapshot-what-gets-published #what-a-snapshot-contains
  - sharing/static-permissions #permissions
  - sharing/what-a-static-share-always-exposes #what-anyone-with-the-link-can-see
  - sharing/hide-devices #hide-devices
  - sharing/download-can-t-stop-recording #download-and-recording
  - sharing/publishing-again-replaces-the-old-link #publishing-again
  - sharing/snapshots-never-expire-only-this-device-can-remove-them #links-dont-expire
  - sharing/publish-progress-cancel-and-rate-limits #publishing-takes-a-moment
---

A static link is a snapshot of your project that anyone can open in a browser and play, even when your computer is off. This page explains what gets published, the permissions, and how to replace or remove a link. For working together live, see [live sessions](/docs/sharing/live-sessions) instead.

## publish a snapshot

1. Click **share** in the top bar and pick the **static snapshot** tab.
2. Set the permissions (below).
3. Click **publish snapshot**.
4. When it's done, click **copy** and send the link.

The link opens in [the share viewer](/docs/sharing/share-viewer).

## what a snapshot contains

Ethereal renders the mix (and, by default, every audio and MIDI track on its own) as MP3 at 320 kbps, together with a read-only view of the project, and uploads it to a link of the form `ethereal.ws/s/…`.

It's frozen: the edits you make afterwards are not in it until you publish again.

## permissions

| switch | default | what it does |
|---|---|---|
| **allow editing (they can open a copy)** | off | listeners get **open a copy to edit**: their own private copy of the project. |
| **allow listening to tracks separately** | on | listeners can solo and mute each track. Off: only the mix is rendered and uploaded, so it publishes faster. |
| **allow download** | off | listeners can download the mix, each track (if the previous switch is on) and the project, in lossless FLAC 24-bit. |
| **hide devices** | off | removes every device and plugin from the view and the project copy (below). |

The hints under the switches say exactly what people will be able to do. Changing a switch after publishing turns the button into **publish again**: the switches apply only to the next snapshot.

## what anyone with the link can see

Even with every switch off, the viewer shows the whole arrangement: track names, clips, MIDI notes, automation, mixer values, markers and tempo. Unless you **hide devices**, it also shows your devices and plugins by name, with their parameter values.

It never includes plugin state, the paths of files on your computer, or the chat.

## hide devices

**hide devices** removes every device and plugin chain (with their automation, racks and modulators) from the view and from the downloadable project. MIDI tracks in that project copy are then silent; the rendered tracks still carry the sound.

> [!note]
> Live sessions have their own switch, **hide devices from listeners**: see [live sessions](/docs/sharing/live-sessions#hiding-devices-from-listeners).

## download and recording

Without **allow download** there's no download menu and the server refuses the lossless files. But anyone can still record the audio their browser plays. Don't publish material you can't afford to leak.

## publishing again

**publish again** (or **publish a new snapshot**) uploads a new snapshot with a **new link**, then deletes the old one: the link you sent before stops working. Send the new link around.

**unpublish** deletes the snapshot at once; its link stops working.

## links don't expire

A snapshot stays online until you remove it. The key that lets you replace or unpublish it is stored only in this app on this computer (in this browser, for the web app). On another computer, or after clearing your browser data, you can no longer unpublish or replace it.

> [!warning]
> Keep the app (or browser profile) you published from if you may want to take a link down later.

## publishing takes a moment

The popover shows each step: "rendering the mix…", "rendering the tracks…", "packing the project…", then "uploading… 3 of 12". It may pause with "waiting to continue the upload…" when the server asks it to slow down; it goes on by itself.

**cancel** stops everything and deletes what was already uploaded. Closing the popover doesn't cancel, but closing the project or opening another one does.

To publish to your own storage server instead of the default one, set **static share storage** in **settings › advanced**.

## where next

- [the share viewer](/docs/sharing/share-viewer): what your listeners see.
- [exporting audio](/docs/import-export/exporting-audio): render files instead of a link.
