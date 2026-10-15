---
title: joining a session
order: 30
summary: Open an invite link in the app or in your browser, choose what plays the project, and fix a join that fails.
platform: all
audience: beginner
covers:
  - sharing/joining-open-in-the-app-or-continue-in-browser #opening-an-invite-link
  - sharing/the-join-screen-and-play-with #the-join-screen
  - sharing/why-a-join-fails #when-a-join-fails
  - sharing/the-host-must-be-online-to-join #the-host-must-be-online
---

Someone sent you a link to their project. This page walks you through joining, and what to do if it doesn't work. You don't need an account. For what happens once you're in, see [live sessions](/docs/sharing/live-sessions).

## opening an invite link

Click the link. In a browser you land on a page with two buttons:

- **open in the app**: opens the invite in the Ethereal desktop app, if it's installed. If nothing happens after a moment, the page offers to download the app.
- **continue in browser**: joins right here, in the web app. Turn on **always continue in browser** to skip this page next time.

You can also paste a link inside Ethereal with **join a jam…**: on the welcome screen, in the project menu, in the share popover and in the command palette.

> [!note]
> On a phone, the web app shows its "not available on mobile" screen unless you use the mobile beta: see [ethereal on your phone](/docs/mobile/mobile-beta).

## the join screen

Before anything downloads, the join screen shows who invites you and to which project, whether **you can edit** or **you can listen**, and who else is there. If you haven't chosen a name yet, type one in **join as**.

**play with** chooses which computer plays the project for you:

| choice | what it does |
|---|---|
| **my engine** (the default) | your computer plays the project, like any project of yours. |
| *name*'s engine (remote) | you hear that person's computer instead; yours doesn't load the project. They are asked first. See [listening on a friend's computer](/docs/sharing/listening-together). |
| run an engine for me on *name*'s computer | they start a second engine just for you, with their plugins. They are asked first. |
| **no engine (view only)** | the project opens but nothing plays: for a project too big for your computer. |

Click **join**. Ethereal downloads the project and its audio files, with a progress bar, then opens it. **not now** closes the screen without joining.

If you already have an older copy of the project, the screen says so: it's updated, and any changes you made offline are kept as a separate "(local copy)" project.

## the host must be online

Projects live on the host's computer, not in a cloud. Nobody can join while the host's Ethereal is closed: the join screen says "the host's Ethereal is closed" and waits, and the project opens as soon as they're back. If you joined before, **open my offline copy** lets you work in the meantime.

A host on the web app goes offline when they close the tab or their laptop sleeps.

## when a join fails

The join screen says **can't join** and why:

| message | what to do |
|---|---|
| this invite link was reset or is no longer valid | ask the host for a new link. |
| the invite link is damaged | the link was cut or changed when it was copied: copy it again, whole. |
| the session is full | 16 people are already online. Try again later. |
| this project has reached its limit of 64 members | the project remembers 64 people who joined. Ask the host to remove someone from the people list. |
| too many people are joining this project right now | the sharing service limits joins for a moment. Try again in a minute. |
| update Ethereal to join this project | you and the host run different versions. Update the desktop app (or ask the host to). |
| couldn't reach the host's computer (network) | a strict network (office, school, some mobile carriers) blocks direct connections. Try again, or ask the host to set up a relay (TURN) server in **settings › advanced**. |
| can't reach the sharing service | check your internet connection and try again. |
| the host's identity could not be verified | don't continue; ask the host to send the link again through another channel. |
| this project is too large to join | the project's audio is too big to transfer. Ask the host to remove unused audio. |

## where next

- [live sessions](/docs/sharing/live-sessions): what syncs, chat, notes and leaving.
- [listening on a friend's computer](/docs/sharing/listening-together)
