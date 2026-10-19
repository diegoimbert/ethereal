# Changelog

The user-facing changes in each release. The full notes for a version are in
[`docs/release-notes/`](docs/release-notes) and on its
[GitHub Release](https://github.com/diegoimbert/ethereal/releases), which also lists every
merged pull request. Versions follow [`VERSION`](VERSION); see
[docs/RELEASING.md](docs/RELEASING.md).

## Unreleased

Changes on `main` since 0.6.0. They are summarized here when the next version is released.

## 0.6.0 (2026-10-15)

[Release notes](docs/release-notes/0.6.0.md) ·
[GitHub Release](https://github.com/diegoimbert/ethereal/releases/tag/v0.6.0)

Desktop users on 0.5.1 or earlier must reinstall once from
[ethereal.ws/download](https://ethereal.ws/download); updates install from the app after that.

- **Live sharing:** more reliable joins with clear errors, a connection test, a production
  relay (TURN) server, solo and mute for listen guests, and following a peer in the piano roll.
- **Static shares:** a shared link opens the project read-only in the app, with MP3 listening,
  FLAC downloads and stems.
- **Mobile:** a touch interface for phones and touch-only tablets.
- **Docs and tours:** user documentation at ethereal.ws/docs (English and French) and guided
  tours on the welcome screen.
- **Plugins:** one-click installs from a signed catalog of open-source plugins, format filters,
  and VST2/VST3 editors on Linux.
- **Editing:** automation lanes edit like the arrangement, return tracks, named undo steps,
  bounce to new tracks, a shortcuts editor and snapshots.
- **Import and export:** MIDI import and export, MP3 export, Ableton Live Set export, and many
  import fixes for Ableton Live, FL Studio, REAPER and DAWproject files.
- **Languages:** the whole app in French.
- **Web app:** the open project is in the URL, one editing tab at a time, persistent storage,
  and a download page.
- **Reliability, security and performance:** output device loss recovery, no lost recordings or
  stuck notes, a Content-Security-Policy, size limits on imported files, stricter share
  checks, and lighter rendering.

## 0.1.0 to 0.5.1

No hand-written release notes exist for these versions; their tags are in this repository.

| Version | Released |
| --- | --- |
| 0.5.1 | 2026-10-11 |
| 0.5.0 | 2026-10-08 |
| 0.4.0 | 2026-10-05 (the built-in desktop updater) |
| 0.3.0 | 2026-10-04 |
| 0.2.1 | 2026-10-03 |
| 0.2.0 | 2026-10-03 |
| 0.1.0 | 2026-09-27 (first release) |
