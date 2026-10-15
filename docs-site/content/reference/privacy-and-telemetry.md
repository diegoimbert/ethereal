---
title: privacy and telemetry
order: 40
summary: What Ethereal sends, what it never sends, and how to turn each part off.
platform: all
audience: everyone
covers:
  - first-run/privacy-notice-on-first-run #the-first-run-notice
  - app-shell/telemetry-is-on-by-default-and-how-to-turn-it-off #crash-reports-and-usage-statistics
---

Ethereal sends a few kinds of data, each for one reason and each with its own switch. This page lists them, what's in them, and how to turn them off. The full technical list is in [PRIVACY.md](https://github.com/diegoimbert/ethereal/blob/main/docs/PRIVACY.md).

## what is never sent

- Your projects: no project file, track or clip names, notes or automation.
- Audio of any kind.
- File names and paths. Your home folder is removed from error messages and logs.
- Your name, your email address (unless you type it into an issue report) or your IP address.
- Share links, invite keys and room ids.

## the first-run notice

The first time Ethereal runs, a card in the bottom left, **help improve ethereal**, tells you that anonymous crash reports and usage statistics are on. It has a **learn more** link, **settings** (opens the privacy settings) and **ok**. Either button, or the ×, closes it for good. It doesn't block anything: you can keep working while it's there. It only appears when something would actually be sent.

## crash reports and usage statistics

Both are **on by default**. Turn each off in **settings › privacy**:

- **Send crash reports**: when the engine, the app or a sandboxed plugin crashes, the error message and where in Ethereal's code it happened. For a plugin crash, the plugin's id, format and version.
- **Send anonymous usage statistics**: counters, sent in batches, such as "a session started", "a project with 9 to 32 tracks loaded", "an export to WAV succeeded" or "the engine stalled because of a plugin". Each is a name, a couple of short labels and a number.

Turning a switch off also deletes anything of that kind still waiting to be sent.

You are a random **install id** to us, not an account or a person. **Settings › privacy › reset install id** replaces it: nothing sent before can then be linked to what you send next. Every payload also carries the app version, the operating system family, the cpu type and whether it's the desktop or the web app.

### see exactly what's sent

**Settings › privacy › view what's sent** shows the last 50 payloads exactly as they left this device, newest first. The list stays on your device.

### when nothing is sent at all

- [[web]] When your browser has **Do Not Track** or **Global Privacy Control** on.
- [[desktop]] When `DO_NOT_TRACK=1` is set in the environment.
- In development builds.

When telemetry is off for one of these reasons, the privacy settings say why.

## news

A few seconds after launch, Ethereal may show an announcement from the author in a pop-up. To pick the right announcements it sends only the app version, desktop or web, and your operating system family: no install id and nothing about your project. The service keeps nothing about who asked. Turn it off with **settings › privacy › check for news**. See [reporting issues](/docs/reference/reporting-issues#news) for how news pop-ups behave.

## things you send yourself

These are only sent when you ask, and you see what's in them first:

- **Report an issue**: your description, the category, your email if you type one, and (unless you untick **attach diagnostics**) a short list about your setup. See [what a report attaches](/docs/reference/reporting-issues#what-a-report-attaches).
- **Send diagnostics…**: system info, a summary of the open project and, on desktop, the end of the engine log. See [send diagnostics](/docs/reference/reporting-issues#send-diagnostics).

Do Not Track doesn't block these: you send them on purpose.

## other services

- **AI assistant**: if you set up an AI provider with your own key, your messages go to that provider with an overview of the project (track, clip, device and section names, tempo, the mix) and whatever the assistant reads with its tools, such as notes and parameters. No audio. It asks before the first message to each provider. The conversation is kept on your computer with the project until you clear it. See [the AI assistant](/docs/advanced/ai-assistant).
- **Sharing**: in a live session, the project, chat and edits travel encrypted directly between the computers. The sharing service only introduces them and never sees names or content. The people in a session can see each other's IP address. See [live sessions](/docs/sharing/live-sessions).
- **Updates** [[desktop]]: the desktop app checks for new releases. See [settings](/docs/advanced/settings).

## where the data goes

Crash reports, statistics, diagnostics and issue reports go to a service run by the Ethereal maintainers. Only they can read them, and nothing is sold or shared. Diagnostics are deleted after 90 days and issue reports after a year. The details, including how long each kind is kept, are in [PRIVACY.md](https://github.com/diegoimbert/ethereal/blob/main/docs/PRIVACY.md).

## where next

- [Reporting issues](/docs/reference/reporting-issues).
- [Settings reference](/docs/advanced/settings).
- [FAQ](/docs/reference/faq).
