---
title: listening on a friend's computer
order: 40
summary: Hear another person's engine in a session, run an engine on their computer, or join with no engine at all.
platform: all
audience: everyone
covers:
  - collaboration/listen-on-a-friend-s-computer #listening-on-someone
  - collaboration/listening-needs-the-other-person-s-yes #they-say-yes-first
  - collaboration/host-s-listening-settings #your-listening-settings
  - collaboration/listening-latency #latency
  - collaboration/play-with-no-engine-view-only #no-engine-view-only
  - collaboration/run-an-engine-for-me-on-someone-s-computer #running-an-engine-on-their-computer
---

In a [live session](/docs/sharing/live-sessions) everyone normally plays the project on their own computer. This page covers the other options: hearing a friend's computer, borrowing their computer to run an engine for you, or opening a project with no engine.

## listening on someone

Open the share popover, find the person in the people list, and choose **listen on** followed by their name from their **…** menu. You can also right-click their avatar in the top bar. You then hear their engine's output, streamed to you, with a shared playhead: a status under the top bar says "listening to" and their name, and **×** stops.

It's useful when they have a plugin you don't, or a faster computer. While you listen:

- your play, stop, loop and playhead control *their* engine, if they allow it (below);
- you hear their mute and solo, and your mute and solo clicks change theirs;
- you can't record: Ethereal says "stop listening to record".

Guests who join with a listen link start listening to the host automatically. To turn that off, switch off **automatically listen to the host when joining with a listen link** in **settings › sharing**. In the share popover, listen guests have **stop listening** and **listen again**.

## they say yes first

The person you want to hear gets "*name* wants to listen to your engine" with **allow** and **deny**. With no answer in 30 seconds, the request is refused. Until they answer, you see "connecting to…". Allowing lasts for the rest of the session, and they can stop or revoke it at any time.

## your listening settings

In the share popover, under **listening and visibility**:

- **let others listen to my computer** (on by default).
- **listeners can play, stop and move the playhead** (on by default). Turned off, a listener's play, stop and mute or solo clicks are ignored.
- the lists of who is listening to you and who is allowed, each with **stop** or **revoke**.

While someone listens, a badge such as "1 listening" shows in the top bar. When several people control the same engine, the last command wins: there's no locking.

## latency

What you hear while listening is a little behind the other computer, roughly 60 to 200 milliseconds (network, buffering and your audio output). Ethereal moves your playhead to match what you hear. That's fine for mixing and giving feedback, not for playing along in time.

## no engine (view only)

Choose **no engine (view only)** in **play with** (on the join screen, or in the share popover during a session) to open a project that's too heavy for your computer. You can look around, and edit if you have the right, but play and record are disabled and waveforms aren't drawn. A **no engine — view only** badge in the top bar has **use my engine** to switch back.

If your engine can't load the project when you join, Ethereal falls back to this mode by itself and the badge explains why.

## running an engine on their computer

**run an engine for me on** followed by a name (in **play with**) asks that person's desktop app to start a second engine just for you, with their cpu and their plugins. Only you hear it; they don't. This needs the other person to use the desktop app.

They're asked first: "*name* wants to run an engine on your computer for them (uses your cpu and plugins; you won't hear it)". Under **listening and visibility** they can turn off **let others run an engine on my computer**, set how many may run at once (2 by default, up to 4), see each engine's cpu use and **stop** it.

## where next

- [live sessions](/docs/sharing/live-sessions)
- [joining a session](/docs/sharing/joining)
- [the engine server](/docs/advanced/engine-server): an engine on another machine, outside sessions.
