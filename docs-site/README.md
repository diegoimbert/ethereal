# User docs (/docs)

The documentation users read at `/docs` on the web app (and over the app on the desktop:
project menu › **documentation**). It's written in Markdown here and bundled into the app at
build time (`ui/src/features/docs`). The developer docs in `/docs/*.md` are separate and aren't
shown to users.

- `content/`: the pages (format below).
- `inventory.json`: everything users need explained (the "explain" inventory, #665). Each item
  has an id (`<area>/<slug>`). An item that no longer applies keeps its entry with
  `"removed": "<why>"`.
- `pending.txt`: inventory items and pages not written yet. It's empty once the docs are complete.
- `coverage.json`: which page and heading covers each item (generated). Its percentage
  measures the inventory only: pages with no `covers` lines (the FAQ, the glossary, the
  example songs, mixing, MIDI files…) aren't counted, and a feature
  missing from the inventory isn't either.
- `content-fr/`: the French translation, page by page (docs/LOCALIZATION.md "Docs"). When you
  change an English page, change its French page the same way: a heading you add, remove or
  rename there (or a `covers` line) makes the translation stale, which the tests catch. So
  does a paragraph, list item, table row or link that one has and the other hasn't.

Checks (`ui/src/features/docs/*.test.ts`): every live inventory item is covered by a page
heading or is pending; links, anchors, shortcuts and images resolve; titles and headings are
lowercase. After changing pages, refresh the report:

```
ETHER_DOCS_UPDATE=1 pnpm --filter @ethereal/ui exec vitest run src/features/docs/coverage.test.ts
```


## Files
- Pages: `docs-site/content/<section>/<page>.md`. The URL is `/docs/<section>/<page>`. The file name is the slug: lowercase with hyphens.
- Sections (dirs, made by the shell PR, not by you): welcome, basics, making-music, sounds-and-plugins, sharing, import-export, advanced, mobile, reference.
- Images (only where a screenshot really helps; under 200 KB): `docs-site/content/<section>/images/<name>.png|webp`.

## Front-matter (required)
```
---
title: the piano roll
order: 30
summary: Draw, select, move and quantize notes, and edit velocity.
platform: all
audience: beginner
covers:
  - midi-editing/draw-a-note-double-click-empty-space-or-turn-on-draw-mode #drawing-notes
  - midi-editing/new-notes-are-one-grid-step-long #drawing-notes
---
```
- `title`: lowercase, as the nav label and page heading (the app's UI style).
- `order`: a number. The pages of a section are sorted by it.
- `summary`: one sentence in sentence case. It is shown as the page's lead ("what you'll learn") and in search.
- `platform`: `all`, or a comma list of `desktop`, `web`, `mobile`. Shown as badges.
- `audience`: `beginner` | `everyone` | `advanced`.
- `covers`: each line is `- <inventory item id> #<anchor>`. The anchor is the id of a heading on this page that explains that item. The coverage test checks that every id exists and every anchor exists on the page.

## Markdown subset (only this renders)
- Headings: `##` and `###` (and `####`). There is no `#` in the body, because the title is the h1. Headings are lowercase. Ids are derived automatically: inline markup is stripped, the text is lowercased, everything outside `a-z 0-9 space -` is removed, spaces become `-` and repeated `-` collapse to one. `## take lanes and comping` → `#take-lanes-and-comping`. To set an explicit id, end the heading with `{#my-id}`.
- Paragraphs, `**bold**`, `*italic*`, `` `code` ``, `[text](url)`.
- Lists: `- item` and `1. item`, nested by 2 spaces. A continuation line is indented by 2 spaces.
- Callouts: a blockquote whose first line is `> [!tip]`, `> [!note]` or `> [!warning]`, followed by `> text` lines. A plain `> ...` renders as a note.
- Tables: GFM pipe tables (`| a | b |`, then `|---|---|`, then rows).
- Fenced code blocks with ```.
- Images: `![alt text](images/file.png)` on a line of its own.
- Internal links: `/docs/<section>/<page>` or `/docs/<section>/<page>#anchor`. The test checks that every internal link resolves (page and anchor).
- Ethereal extensions (inline):
  - `[[action:<keymap action id>]]`: renders that action's default shortcut from the real keymap (`ui/src/features/keymap/registry.ts`, BUILTIN_ACTIONS, plus `midi.computerKeyboard`), with ⌘/⌥/⇧ on a Mac and Ctrl/Alt/Shift elsewhere. Use it for every shortcut that is a keymap action (e.g. `[[action:edit.undo]]`, `[[action:transport.play]]`, `[[action:palette.open]]`). Never write a chord that is a keymap action by hand.
  - `[[key:<chord>]]`: a key or chord that is not a keymap action (modifier gestures, keys inside a field). The chord uses the keymap syntax: `Mod` (⌘ on a Mac, Ctrl elsewhere), `Ctrl`, `Alt`, `Shift`, joined with `+`, e.g. `[[key:Alt]]`-drag, `[[key:Mod+Shift]]`-drag, `[[key:Enter]]`, `[[key:Shift+ArrowUp]]`.
  - `[[desktop]]`, `[[web]]`, `[[mobile]]`: an inline platform badge (e.g. after a heading: `## recording [[desktop]]`. Badges are not part of the heading id).
  - A line with only `[[shortcuts]]`: the generated table of every shortcut. This is reserved for reference/keyboard-shortcuts.

## Writing rules
- Write for someone who has never used Ethereal. Use second person and plain words, with short paragraphs. No marketing, no filler and no em dashes: use colons, commas or full stops.
- Prose is in normal sentence case. Headings and UI names are lowercase, as the app shows them. Put UI labels in **bold**, written exactly as the app shows them (e.g. **export audio…**, **versions…**, the **+** button).
- Each page: the summary is the lead. The first paragraph says what the page covers and what you need first. End with `## where next`: 2–4 links onward.
- Beginners first: put the must/beginner items early on the page. Advanced details go lower or under `### details`-style subheadings.
- Platform differences: use the badges and say plainly what doesn't work where (web vs desktop vs mobile).
- Use callouts for warnings that can lose work (e.g. "web projects live only in this browser"; "delete is permanent"; "flatten can't be undone").
- Describe what the app does NOW on origin/dev. Verify every item against the code: the inventory's `refs` (in /home/diegoimbert/orca/projects/ethereal/.orchestra/explain/inventory.json) are hints from a few hours ago, and features change constantly. Read the actual components/handlers, labels and tests. Never document something you haven't confirmed in code.
- If an item no longer applies (feature removed or changed), don't cover it. Report it as removed with a reason (see the workflow below). If the app's behaviour looks like a bug or a bad UX that should be fixed rather than documented, document current behaviour neutrally if users will meet it, and list it in your report under "needs owner input".
- Don't invent features, settings or limits. Don't promise future work, except to say "in development" where the app itself says beta/soon.
