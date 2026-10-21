---
title: safe mode
order: 80
summary: Open a project with its plugins turned off, when a plugin crashes or hangs it.
platform: desktop
audience: advanced
covers:
  - first-run/safe-mode-open-without-plugins #opening-without-plugins
---

A broken plugin can crash Ethereal or freeze it as soon as a project opens. Safe mode opens the project with every plugin turned off, so you can get into it, find the culprit and save your work.

## opening without plugins

There are two ways in:

- On the welcome screen, right-click a recent project and choose **open without plugins**.
- After a crash, the recovery dialog lists the projects that were open and offers **open without plugins** for each.

The project opens with every plugin bypassed: it makes no sound and doesn't run, but its settings are kept. Built-in devices work as usual.

## while in safe mode

- The status bar (at the bottom) says **plugins disabled (safe mode)**.
- Each plugin shows **safe mode · bypassed**, with a **⟳** button that loads just that plugin. Loading them one by one is how you find the one that crashes.
- **load plugins** next to it loads them all and leaves safe mode.

Opening in safe mode doesn't change the saved project: each plugin keeps its state.

## where next

- [plugins](/docs/sounds-and-plugins/plugins): sandboxing, crashes and missing plugins.
- [versions and backups](/docs/basics/versions-and-backups): crash recovery.
- [troubleshooting](/docs/reference/troubleshooting)
