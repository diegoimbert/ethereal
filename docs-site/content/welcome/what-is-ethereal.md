---
title: what is ethereal
order: 10
summary: What Ethereal is, the desktop app vs the web version vs the phone beta, and how to get new versions.
platform: all
audience: beginner
covers:
  - first-run/what-the-web-version-can-t-do #what-the-web-version-cant-do
  - remote-engine/web-vs-desktop-plugins #what-the-web-version-cant-do
  - remote-engine/web-vs-desktop-recording-and-midi #what-the-web-version-cant-do
  - remote-engine/web-hosting-and-dedicated-engines #what-the-web-version-cant-do
  - remote-engine/web-phones-and-big-projects #phones-and-tablets
  - first-run/the-open-on-desktop-band #the-open-on-desktop-band
  - first-run/phones-and-touch-only-tablets-are-blocked #phones-and-tablets
---

Ethereal is a music app (a DAW, a digital audio workstation): you make songs out of tracks of notes and audio, play them through instruments and effects, record, mix and export.

This page explains the three ways to run it and what each one can do. If you just want to start, go to [your first loop](/docs/welcome/first-beat).

## three ways to run it

| | where | best for |
|---|---|---|
| **the desktop app** [[desktop]] | macOS, Windows and Linux, from [the releases page](https://github.com/diegoimbert/ethereal/releases) | everything: plugins, recording, MIDI learn, your audio interface |
| **the web version** [[web]] | a browser on a computer, at ethereal.ws | trying Ethereal, quick ideas |
| **the mobile beta** [[mobile]] | a phone or a tablet's browser | listening and small edits on the go (in development) |

All three are the same app with the same projects format and the same built-in instruments and effects. The engine that makes the sound runs on your own computer in each case: in the desktop app natively, in the web version inside the browser.

## what the web version can't do

The browser limits what a web page can do with your computer, so some features need the desktop app. Where you meet one, the app says so ("… needs the desktop app"):

- **Plugins** (CLAP, VST3, AU): the device library says "plugins are available in the desktop app". The built-in instruments and effects all work. See [plugins](/docs/sounds-and-plugins/plugins).
- **Recording** audio or MIDI from your inputs. See [recording](/docs/making-music/recording).
- **MIDI learn**, and hardware routing with latency measurement (external instruments and effects).
- **Choosing your audio devices**: in the browser, your system picks the output. Settings › audio says so.

> [!warning]
> Web projects live only in this browser, on this computer. Read [where your projects live](/docs/welcome/where-your-projects-live) before you put hours into one.

## the open on desktop band

On a computer, the web version shows a band at the top: **ethereal works best in the desktop app**, with **open on desktop**. That button opens the desktop app if it's installed. Next to it, **discord** opens the Ethereal community's Discord in a new tab (the welcome screen has the same link, as a Discord icon).

Opening the desktop app doesn't move your web projects to it: the two keep separate projects (see [moving projects](/docs/basics/moving-projects)). The **×** hides the band until you reload the page.

## phones and tablets

On a phone, or a tablet without a mouse or trackpad, the web version shows "ethereal is not available on mobile — try it on desktop", with **copy link** (to open the same link on a computer) and **try the mobile beta**. The beta is a touch version of the app, in development: see [Ethereal on your phone](/docs/mobile/mobile-beta).

A tablet with a keyboard and a trackpad counts as a computer. These docs work on phones too.

Large projects are heavy for a browser: the desktop app has more room for many tracks and devices.

## getting the desktop app

Installers for macOS, Windows and Linux are on [the releases page](https://github.com/diegoimbert/ethereal/releases).

## updates

To update the desktop app, install the new version from [the releases page](https://github.com/diegoimbert/ethereal/releases). Your projects stay where they are.

The web version is always the latest one: reload the page.

## where next

- [Where your projects live](/docs/welcome/where-your-projects-live): the one thing to know before you start.
- [Your first loop](/docs/welcome/first-beat): make some music in five minutes.
- [A tour of the screen](/docs/basics/the-screen): what each part of the window is for.
