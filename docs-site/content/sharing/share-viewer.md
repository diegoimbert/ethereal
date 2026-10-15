---
title: the share viewer
order: 60
summary: What people see when they open a static link, how they listen to single tracks, and how they open their own copy.
platform: web, mobile
audience: beginner
covers:
  - sharing/the-read-only-viewer #a-read-only-ethereal
  - sharing/solo-and-mute-in-the-viewer #listening-to-single-tracks
  - sharing/open-a-copy-to-edit #opening-a-copy-to-edit
---

This page is for whoever opens a static share link (an `ethereal.ws/s/…` address), and for the owner who wants to know what listeners get. To publish one, see [static share links](/docs/sharing/static-links).

## a read-only ethereal

The link opens the project in Ethereal's own editor, in your browser, read-only. A band at the top shows the song's name and "shared with Ethereal · read-only". Nothing needs installing, and no account is needed.

You can play, stop, loop and click on the ruler to move the playhead, and look around: the arrangement, clips in the piano roll and the devices. Nothing you do changes the song, and the keys that would edit are ignored. What you hear is the rendered audio of the snapshot: MIDI isn't played live.

It works on phones too, with simpler track headers and transport.

## listening to single tracks

If the owner allowed it, each track's solo and mute buttons work: they play that track's own audio. As in the app, a solo is exclusive; [[key:Mod]]-click adds more solos. **back to the full mix** returns to the mix.

The first solo or mute downloads every track ("loading the tracks… the full mix plays until they are ready"). Track meters move once the tracks are loaded.

When the band says "the owner shared the mix only.", only the mix was published and solo and mute don't do anything.

The band may also say "the owner hid the devices.": the device chains were removed from this snapshot.

## downloading

If the owner allowed downloads, a **download** menu offers the mix (lossless), the project file and, when tracks were shared, each track.

## opening a copy to edit

If the owner allowed editing, the band has **open a copy to edit**. It starts the Ethereal web app and imports the project as your own private project, stored in this browser. Your changes never go back to the owner, and the owner's later changes never reach your copy.

> [!note]
> Your copy lives in this browser only, like every web project: see [where your projects live](/docs/welcome/where-your-projects-live). On a phone, opening a copy needs the mobile beta.

## where next

- [static share links](/docs/sharing/static-links)
- [what is ethereal](/docs/welcome/what-is-ethereal)
