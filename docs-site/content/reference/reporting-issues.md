---
title: reporting issues
order: 50
summary: Send a bug report or an idea from inside the app, add diagnostics when you're asked for them, and find past news.
platform: all
audience: everyone
covers:
  - app-shell/report-an-issue #report-an-issue
  - app-shell/what-an-issue-report-attaches #what-a-report-attaches
  - app-shell/send-diagnostics-ticket-for-a-bug #send-diagnostics
  - app-shell/debug-panels-advanced #debug-panels
  - first-run/news-pop-up-after-launch #news
---

Something broke, something's missing, or you have an idea? Tell us from inside the app. You don't need an account. Before you write, a quick look at [troubleshooting](/docs/reference/troubleshooting) may already have the answer.

## report an issue

1. Click **report an issue** at the right end of the thin status bar at the bottom of the window. [[mobile]] On the mobile version, it's in the project menu.
2. Pick a category: **bug**, **missing feature**, **idea / suggestion**, **performance**, **crash** or **other**.
3. Describe what happened in **what happened?**. This is the only required field, up to 4000 characters. For a bug, say what you did, what you expected and what happened instead.
4. Add your email if you'd like a reply. Leave it empty to stay anonymous.
5. Press **send** (or [[key:Mod+Enter]] in the description).

You get a report id that starts with `R-`. Keep it if you want to refer to your report later. If sending fails, the form keeps what you typed and offers **try again**. You can send a limited number of reports per hour; if you hit the limit, try again later.

If the audio engine fails to start, the error screen has its own **report an issue** button.

## what a report attaches

**Attach diagnostics** is on by default. Open **what's attached** in the form to see the exact list before you send. It holds:

- the app version, and whether it's the desktop or the web app;
- your operating system, number of CPUs, memory and language;
- the audio setup: backend, sample rate, buffer size, latency, the number of dropouts, and whether the engine is running;
- the size of the open project: the number of tracks, clips, notes, devices and media files.

It never contains the project itself, names, paths or keys. Turn **attach diagnostics** off to send only your text. Do Not Track doesn't block a report, because you send it on purpose. See [privacy and telemetry](/docs/reference/privacy-and-telemetry).

## send diagnostics

When we ask you for more detail about a bug, use **send diagnostics…**: in the command palette ([[action:palette.open]], then type "diagnostics"), or **settings › privacy › send diagnostics…**.

The dialog lets you add an optional description and shows **what will be sent** before you send it:

- system info: platform, CPUs, memory, language, audio backend, sample rate, buffer size and dropouts;
- a summary of the open project: counts, and the names, formats and versions of the plugins it uses;
- [[desktop]] the end of the [engine log](/docs/reference/troubleshooting#the-engine-log).

Paths, emails and IP addresses are removed. You get a ticket id like `D-7K3QX9`, with a **copy** button: quote it in your report. Diagnostics are kept for 90 days. This needs telemetry to be available: if it's off (for example with Do Not Track), the dialog says diagnostics can't be sent from here.

## debug panels

Debug panels are floating windows that show what a part of the app does inside, for developers. When some are available, they're listed in **settings › advanced › debug panels** and in the command palette as **debug: …**, and Ethereal remembers which ones are open and where. There are none in the current version, so you won't see that section.

## news

About four seconds after launch, Ethereal may show an announcement from its author in a pop-up. Close it however you like (**close**, the ×, [[key:Escape]]) and it won't show again. If there are several, **next** moves to the next one and marks the current one as seen.

To read past announcements again, choose **news…** in the project menu or the command palette. To stop the pop-ups, turn off **settings › privacy › check for news**: Ethereal then doesn't ask for news at all. See [privacy and telemetry](/docs/reference/privacy-and-telemetry#news).

## where next

- [Troubleshooting](/docs/reference/troubleshooting).
- [Privacy and telemetry](/docs/reference/privacy-and-telemetry).
- [FAQ](/docs/reference/faq).
