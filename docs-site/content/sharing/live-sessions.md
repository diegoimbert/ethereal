---
title: live sessions
order: 20
summary: Start a live session, manage who's in, and understand what everyone shares and what stays yours.
platform: all
audience: everyone
covers:
  - sharing/allow-editing-toggle-live #allow-editing
  - sharing/reset-link-vs-remove-vs-stop-sharing #reset-remove-or-stop
  - sharing/quitting-is-not-stopping-sharing #closing-the-app-doesnt-stop-sharing
  - sharing/people-list-roles-and-limits #the-people-list
  - sharing/session-pill-and-view-only-badge #the-session-pill
  - sharing/shared-projects-in-your-project-list #shared-projects-in-your-project-list
  - sharing/privacy-of-live-sharing #privacy
  - collaboration/everyone-plays-on-their-own-computer #everyone-plays-on-their-own-computer
  - collaboration/mute-and-solo-are-per-person #mute-and-solo-are-yours
  - collaboration/what-syncs-and-what-stays-local #what-syncs-and-what-stays-yours
  - collaboration/audio-files-are-sent-to-everyone #audio-files-are-sent-to-everyone
  - collaboration/plugins-don-t-travel #plugins-dont-travel
  - collaboration/undo-only-undoes-your-own-changes #undo-only-undoes-your-own-changes
  - collaboration/live-pointers-and-selections #seeing-each-other
  - collaboration/follow-someone-s-view #following-someones-view
  - collaboration/hide-users-and-notes #hide-users-and-notes
  - collaboration/session-chat #chat
  - collaboration/pinned-notes #pinned-notes
  - collaboration/offline-copies-and-host-going-offline #when-the-host-goes-offline
  - collaboration/local-copy-projects #local-copy-projects
  - collaboration/leaving-a-session #leaving-a-session
  - collaboration/when-sharing-ends #when-sharing-ends
  - collaboration/everyone-must-run-the-same-version #everyone-needs-the-same-version
---

A live session lets other people work on your project with you, in real time, each on their own computer. This page is for the person who shares (the host) and for everyone who joins. If you haven't picked a kind of sharing yet, start with [sharing your music](/docs/sharing/overview).

## start a session

1. Open the project and click **share** in the top bar.
2. On the **live collaboration** tab, click **start live session**.
3. Pick the **can edit** or **can listen** tab, click **copy**, and send the link.

The first time, the popover asks for your name and colour (**you appear as**). Your friends open the link and join: see [joining a session](/docs/sharing/joining).

> [!note]
> Nobody can join while your Ethereal is closed. On the web, closing the tab or letting the laptop sleep takes the session offline too.

## allow editing

The **allow editing** switch in the popover hides the edit link, so only the listen link is offered. It doesn't remove anyone: people who already joined with an edit link can still edit. To take edit rights away, reset the edit link and change or remove those people (below). The switch is saved with the project's sharing settings, so it stays as you left it when you reload or reopen the project. Stopping sharing resets it.

## the people list

The popover lists everyone who has joined the project, online or not, with a dot for who's online now and "last seen…" for the others. The host can change a person's role (**can edit** or **can listen**) and, from the row's **…** menu, **remove from project**.

A session holds at most 16 people online at once, and a shared project remembers at most 64 members. Past that, a join is refused, and the person joining is told which limit was hit ("the session is full", or "this project has reached its limit of 64 members").

## hiding devices from listeners

**hide devices from listeners** in the share popover keeps your devices private from people who joined with a listen link: they get the project without devices, plugins or their settings (and the automation and MIDI mappings that go with them). People with the edit link always see everything. Turning it on or off reconnects the listeners so they get the right version at once.

## reset, remove or stop

There are three levels, from the smallest to the biggest:

| action | where | what happens |
|---|---|---|
| **reset edit link** / **reset listen link** | the **…** menu at the bottom of the popover | the old link stops working. People already in keep their access, because each member has a personal key. |
| **remove from project** | the person's **…** menu in the people list | only that person loses access. They can come back only with a current link. |
| **stop sharing** | the button at the bottom of the popover | every link and every member key stops working. Everyone who joined keeps an offline copy. |

To kick out someone who has a link you sent around, do both: reset the link, then remove them.

## closing the app doesn't stop sharing

Closing Ethereal or opening another project only pauses the session. The links stay valid, the people in the session see "*host* went offline", and when you open the project again the session resumes by itself. Only **stop sharing** ends it.

If you'd rather share again by hand each time, turn off **resume sharing when I open a shared project** in **settings › sharing**.

## the session pill

While a project is shared (or you've joined one), the **share** button becomes a pill with the avatars of the people online and a status:

- **live**: connected, edits sync.
- **connecting…** or **reconnecting…**: on the way, or back after a dropped connection.
- **not joinable** (host): the sharing service can't be reached. People already in stay connected, but nobody new can join.
- the host's name followed by **offline** (guest): you're working on an offline copy (see below).

Click the pill to open the popover. Guests who joined with a listen link also see a **view only** badge.

## everyone plays on their own computer

Each person has their own copy of the project and their own audio engine. Playback is personal: play and stop, the playhead, the loop and the metronome are yours alone. Pressing play doesn't play for the others, and their playback doesn't move yours. The others' playheads show as thin coloured lines with their initials.

To hear exactly what someone else hears, listen on their computer: see [listening on a friend's computer](/docs/sharing/listening-together).

## mute and solo are yours

Mute and solo on tracks and drum pads are personal, like your own headphones: they are never sent to the others. When you join, you start once from the host's mute and solo, then each of you mutes and solos independently. The exception: while you listen on someone's computer, you hear (and control) their mute and solo.

## what syncs and what stays yours

| synced with everyone | stays on your computer |
|---|---|
| tracks, clips, notes, devices and their parameters, automation | play, stop, record and the playhead |
| volume, pan, routing, sends, names, colours, track order | the loop and the metronome |
| tempo, time signatures, markers, warp markers | mute, solo and record arm |
| MIDI mappings, drum pads | your selection and your undo history |
| the chat and pinned notes | your audio settings and preferences |
| the project name and swing | |

## audio files are sent to everyone

When someone imports audio or records a take, the file is sent to everyone before the clip appears. People who join later download all the audio first: that's the progress bar on the join screen. A session holds about 1 GB of audio. A very big project can fail to join with "this project is too large to join".

## plugins don't travel

Plugins are never shared, only their settings. If someone doesn't have a plugin you use, it's bypassed on their side (they hear that track dry or silent), and that never affects you. Changes made inside a plugin's own window (a preset loaded in the plugin, for example) reach the others when that person's project is saved, by hand or by autosave.

To let someone hear your plugins anyway, they can listen on your computer: see [listening on a friend's computer](/docs/sharing/listening-together).

## undo only undoes your own changes

In a session, [[action:edit.undo]] undoes only your own edits, never the others'. If a friend changed the same thing after you, your undo leaves their change in place. Undoing inside something a friend has since deleted is skipped too. So undo can sometimes seem to do nothing: that's someone else's newer change winning.

## seeing each other

In the arrangement you see the others' mouse pointers with their names in their colours, their selected clips and tracks outlined in their colour, and a ring around clips someone is editing. Nothing of this is a glitch, and none of it changes your project.

## following someone's view

Click a person's avatar in the top bar, or choose **follow** and their name in their **…** menu in the share popover. Your arrangement then scrolls and zooms with theirs, and when they open a clip in the piano roll, yours opens it too. Following never changes your playback.

Following stops as soon as you scroll or zoom yourself, press [[key:Escape]], or click the avatar again.

## hide users and notes

Too busy? Turn on **hide users and notes**: in the popover under **listening and visibility** (host) or **visibility** (guests). It hides the others' pointers, playheads, selections and pinned notes on your screen only. Edits, the chat and the people list keep working.

## chat

During a session, a **chat** tab appears in the left rail. [[action:collab.focusChat]] jumps to the message field. [[key:Enter]] sends and [[key:Shift+Enter]] starts a new line; a message holds up to 2,000 characters. Guests with a listen link can chat too.

Messages are saved in the project (the last 2,000) and show again in the next session. They can't be edited, deleted or undone.

## pinned notes

Right-click an empty spot in the arrangement (a lane or the ruler) or in the piano roll grid and choose **leave a note** to pin a comment there. Click a dot to read it. Its menu has **edit note**, **resolve note** (or **reopen note**) and **discard note**, and you can drag a dot to move it.

Anyone in the session can edit, resolve or discard any note, not only their own. A project holds up to 500 notes. Notes also work outside a session, as reminders to yourself.

## when the host goes offline

If the host closes Ethereal, guests keep working on an offline copy, with a banner: "*host* is offline. You're working on an offline copy. Changes sync when they're back; keep Ethereal open." A dropped connection reconnects by itself.

> [!warning]
> Keep Ethereal open while the host is away. Your waiting changes sync when the host comes back.

## "local copy" projects

If you edited a shared project while offline and the session's version moved on without you, your work isn't merged into it. When you reconnect, your version is saved as a separate project named "*song* (local copy)" with a **local copy** badge, and the session's version opens. The toast says "offline changes kept". Nothing is lost: open the local copy and bring over what you need by hand.

## leaving a session

Guests: **leave** at the bottom of the popover. Your copy stays on this computer and appears in your projects with a **from** badge naming the host; opening it again reconnects.

Opening, creating or importing another project while you're in a session asks first: "leave the *song* session?", with **leave & open** (or **leave & create**…). This also applies to the host.

## shared projects in your project list

In the project list, shared projects carry a badge with the last people seen:

- **shared**: you host it. Its menu has **copy invite link** and **stop sharing…**.
- **from** and the host's name: your copy of someone else's project. Its menu has **reconnect** and **make a private copy…**.
- **sharing ended**: the host stopped sharing it.
- **local copy**: see above.

**make a private copy…** turns the project into a normal one that no longer syncs with the host. Duplicating a shared project or saving it under another name never carries the sharing.

## when sharing ends

When the host stops sharing or removes you, you get "*host* stopped sharing *song*. Your copy stays on this computer." The project shows **sharing ended** in your list. Use **make a private copy…** to keep working on it on your own.

## everyone needs the same version

Everyone in a session must run the same version of Ethereal. If yours is different, the join fails with "update Ethereal to join this project". The web app is always the latest version. On the desktop, update the app (see [getting the desktop app](/docs/welcome/what-is-ethereal)).

## privacy

The project, the chat and every edit travel encrypted, directly between computers (or through a relay that only sees encrypted data). The sharing service only introduces the computers to each other: it never sees names, the project or messages. Like any peer-to-peer app, the people in a session can see each other's IP address. To hide yours, turn on **hide my IP (relay only)** in **settings › advanced**; it connects through a relay server only.

## where next

- [joining a session](/docs/sharing/joining)
- [listening on a friend's computer](/docs/sharing/listening-together)
- [undo and history](/docs/basics/undo-and-history)
