# Guided tours (in-app tutorials)

The welcome screen's **get started** items are guided tours of the real UI (refs the
tutorial format): the welcome screen hides, the screen is dimmed except the part to look at,
and a bubble explains it. Code: `ui/src/features/tutorials/**`.

| Tour | id | Project it makes | Ends with |
| --- | --- | --- | --- |
| Make your first track | `first-track` | "my first track" (empty) | "open the guide" or finish |
| Jam with your friends | `jam` | "jam tutorial" (the starter project) | "start a session now" or finish |
| Record audio | `record-audio` | "record audio tutorial" (empty) | finish |
| Bring a project from another DAW | `import-daw` | "import tutorial" (empty) | "pick a file now" or finish |
| Example beats (×4, below) | `example-<slug>` | the example ("house beat", …), from its template | "open the guide" or finish |

**Make your first track** (#14, `tours/firstTrack.tsx`) needs no sample: a MIDI track (Add
track, then MIDI track), its instrument (the picker that opens under it: the built-in synth
it has, or the poly synth), a clip (a double-click on the empty lane), opened in the piano
roll (a double-click on the clip), three notes drawn there, then play and stop. It ends with
"open the guide": `welcome/first-beat` ("your first loop"), which goes on with a second
instrument, an effect and the mix. When the default sample pack ships, a step that
drags a sample in can join it.

## Example beats

The welcome screen's **examples** (under get started; Help: the palette's and the project
menu's **example beats** bring the welcome screen back) are short loops that sound good at once
and are simple to rebuild: **house beat**, **hip-hop loop**, **lo-fi chords** and
**simple bassline**.

- **The projects** are factory project templates (`factory/example-<slug>`), built in code by
  `crates/ether-controller/src/templates/examples.rs` the way a user builds them (tracks,
  devices, factory presets, pads, clips, notes, tempo, swing, loop), so they also show in the
  **new project** form's "start from" list. Only built-in devices and factory presets: the drums
  are drum rack pads playing a **poly synth** with one of its drum presets (`poly-synth/kick`,
  `snare`, `clap`, `closed-hat`, `open-hat`). `pad_sound` there is the one place that decides
  what a pad plays: when the default sample pack ships, it gives each drum its sample
  instead, and every example follows. The docs guides then change their "make the drum kit"
  step.
- **Open** (a click on the row) makes a project from the template, named after the example.
- **Show me how** runs the example's tour (`tours/examples.tsx`, one builder for the four):
  the tour's project is made from the template (`TourDef.project: { template }`), it waits for
  play, shows the tempo and loop, the tracks, then for each part its devices (the track
  selected: the inspector) and its clip (opened in the editor drawer), and ends with "open the
  guide".
- **The guides** (`docs-site/content/welcome/example-*.md`, and `content-fr`) rebuild each one
  from an empty project, click by click, with the patterns and notes.
- **Tests**: `examples.rs` (each builds a valid project with its parts), `export.rs`
  `example_projects_render_every_track` (each renders its loop, every track audible, the mix
  unclipped), `poly_synth.rs` `drum_presets_hit_and_decay`, `apps/web/e2e/examples.spec.ts`
  (each opens from the welcome screen and every track is heard on its meter in the wasm
  engine; Help brings the examples back), and the tours in `just e2e-tours`.

## How a tour runs

1. A click on a get-started item (`ui/src/features/project/GetStarted.tsx`) makes the tour's
   project (a number is added when the name is taken), opens it, measures what the host can do
   (`TourEnv`: recording, audio device lists) and starts the tour. In a collaboration session
   it asks first, like any project switch (`guardLeave`).
2. `TourRoot` (mounted once by the desktop shell in `App.tsx`) draws the overlay and the bubble
   (step counter, back, next, exit) and guards input.
3. **Finish** marks the tour done and shows the welcome screen again, where the item has a
   check mark; a click on a done item asks "you already completed this tutorial. do it again?".
   **Exit** (the ×, or Escape then "exit") leaves it undone and shows the welcome screen. A last
   step's **action** ("start a session now", "pick a file now") marks it done and goes on in the
   app instead. The tour's project stays in the user's projects either way.

## Writing a tour

A tour is data (`TourDef` in `types.ts`); put it in `tours/` and list it in `tours/index.ts`
and in `GetStarted.tsx`.

```tsx
export const MY_TOUR: TourDef = {
  id: "my-tour",
  platforms: ["desktop", "web"], // where the welcome screen offers it
  version: 1, // bump when rewritten: the old completion no longer counts
  title: "My tour",
  projectName: "my tour tutorial",
  project: "empty", // or "starter"
  cleanup: () => closeFloating(), // whatever its steps may have left open
  steps: [
    { id: "intro", title: "Hello", body: <p>What this tour shows.</p> },
    {
      id: "open-settings",
      targets: ["settings-button"], // data-tour names
      title: "Open the settings",
      body: <p>…</p>,
      hint: "Click the gear",
      until: () => useAudioSettings.getState().open, // moves on by itself
    },
  ],
};
```

A step:

- **targets**: `data-tour="…"` names (add the attribute to the real element; never a CSS
  class), or a function returning elements (a given track's header). None: a centered bubble.
  Several targets: one hole each.
- **title / body**: short paragraphs or lists (JSX), or a function of the context to branch the
  copy on the platform. **illustration**: a small drawing above the text (`tours/diagrams.tsx`).
  Write copy in sentence case in the source, like the rest of the UI: the base stylesheet shows
  it lowercase.
- **until**: the step waits for the app instead of showing "next"; checked every frame
  against the stores' `getState()` or the DOM. Going forward, a step whose condition already
  holds is skipped; going back, it stays (with "next") until the condition changes again.
  **hint** says what to do.
- **interactive**: whether the targets take clicks (default: only waiting steps).
- **when**: show the step only on hosts where it applies (`ctx.env`).
- **fallback**: shown in a centered bubble with "next" when the targets can't be found for
  `MISSING_GRACE_MS` (1.2 s); without one, a generic "we'll skip it" note. A tour never gets
  stuck.
- **onEnter**: set up what the step needs (open the share window, close the settings).
- **transportKeys**: let space play / stop during the step.
- **actions**: extra buttons that finish the tour and run something.

## Input while a tour runs (`guard.ts`)

- The overlay is an SVG over the app (above dialogs, below popovers and select lists, so a list
  opened from a target is readable); its dimmed area catches presses, the holes let them
  through. The highlight rings are a layer of their own above popovers and menus (a target
  inside a menu, or under one, keeps a visible ring; they take no presses), and the bubble is
  above that (`--eth-z-tour-dim` < `--eth-z-popover` < `--eth-z-tour-ring` <
  `--eth-z-tour-bubble`).
- Window capture listeners (before the app's own handlers) stop presses, clicks, context
  menus, wheel, touch and file drops outside the allowed parts: the bubble, the current
  step's targets when the step is interactive, and pickers opened from them (`role="listbox"`,
  `role="menu"`). A blocked press makes the bubble pulse.
- Keys: Escape asks to exit (inside a picker it closes the picker); Tab cycles through the
  allowed parts; other keys only reach an allowed, focused control, and the app's window and
  document shortcut handlers never see them. The keymap dispatcher and the transport keys are
  paused (`pauseDispatch`) unless the step sets `transportKeys`.
- A press in the bubble is stopped at the bubble, so an open popover (the share window) is not
  closed by it.
- The bubble is a `role="dialog"` with an `aria-live` region: each step is announced, and it
  takes the focus on each step.

## Completion

`progress.ts` keeps `{ "<id>@<version>": finished-at }` in localStorage
(`eth.tutorials.done`), read and written in try/catch. There is no engine-side store for UI
preferences yet (the keymap has its own command); moving this there would make it follow the
user to remote engines.

## Platforms

- **Desktop**: the record tour walks through the driver, output and input devices, sample rate,
  buffer size, the input check, then an audio track's input, arming, the level, recording and
  the first take.
- **Browser** (wasm engine): Settings > Audio can't list devices and the record button is
  disabled. The record tour says so (the system chooses the devices; recording needs the
  desktop app), still shows the settings, the track, the inputs and arming, and skips the
  level / record / stop / take steps.
- **Mobile shell** and the read-only viewer have no tours (the items show "soon").

Each tour declares its `platforms` ("desktop", "web", "mobile"); the welcome screen offers it
only there (`platformOf` in `targets.ts`: the desktop app is "desktop", anything else "web").
Targets that exist only on some hosts (the device fields of Settings > Audio, its
"unavailable" note) are listed in `HOST_TARGETS` (`targets.ts`): a step pointing only at them
needs a `when` guard or a fallback on the other hosts, which the static checks verify. A tour
can't declare "mobile" until the mobile shell mounts `TourRoot`.

## Tests

Tours break quietly: a refactor renames a `data-tour` attribute, a label a step waits on gets
translated, a layer covers the highlight, a waiting step never moves on. Three layers of tests:

- `ui/src/features/tutorials/tutorials.test.tsx` (vitest): the engine. Steps moving on with
  the app's state, forward skip vs back, missing targets and the fallback, completion per
  version, blocked clicks and keys, Escape, and the welcome screen's "do it again?".
- `ui/src/features/tutorials/tours.static.test.tsx` (vitest, in the normal suite, under 2 s):
  the tour definitions against the source. Every target is a `data-tour` name some component
  renders (`data-tour="…"`, or a `tour` prop passed on to it); every attribute selector a tour
  queries (`[role="menu"][aria-label="Project"]`) is set literally somewhere; every step has its
  id, title and body, a waiting step has a hint and a target; bodies render on each declared
  platform and stay short; no visible step points only at targets its platform lacks
  (`HOST_TARGETS`); every tour is on the welcome screen; and the layer order (dialogs < dim <
  popovers < rings < bubble < tooltips).
- `apps/web/e2e/tours.walkthrough.spec.ts` (Playwright, **opt-in**): every tour started from
  the welcome screen and walked to the end like a user, at 1280×800, 1024×640 and 800×600 (web
  build, WebGL arranger). At each step: the targets are found (no fallback), inside the window
  and reachable at their center (nothing covers them, not even the bubble); the ring is the
  topmost element along its edge; the bubble is inside the window and nothing in it is clipped
  or needs scrolling. It then does what the step asks (clicks share, adds an audio track, arms
  it…) and fails, with the step id and a screenshot, if the tour doesn't move on within 10 s.
  At the end the tour is done, the welcome screen is back, and no console error showed. It
  also checks that every tour on the welcome screen has a walkthrough, and that the mobile
  shell offers none (it can't run them). The desktop-only steps (audio devices, recording a
  take) can't run in the browser: the static checks cover their targets and guards.
  - Run it with `just e2e-tours` (about a minute; add `-g jam` for one tour). It isn't in
    `just e2e-web`: the `tours` project only exists with `ETHEREAL_TOURS=1`.
  - Run it **before every release** (docs/RELEASING.md) and **after UI refactors** (the top
    bar, menus, settings, the share window, the arrangement toolbar, recording controls,
    layers and z-index, copy or i18n sweeps).
  - A new tour, or a new waiting step, needs its action in the spec's `TOURS` scripts (the
    run fails with "the walkthrough has no action for it" otherwise).
- `apps/web/e2e/tutorials.spec.ts` (default e2e): the jam tour's guard and back/forth, the start
  of the record tour in the browser, and the ring above an open menu.
