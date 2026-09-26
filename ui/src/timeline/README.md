# `ui/src/timeline`: timeline primitives

Shared building blocks for every time-based view: the arrangement, piano roll, automation
lanes and warp editor. Import everything from `@/timeline`. Musical time is `f64` beats
(quarter notes). Grid math goes through the shared helpers in `@/state/beats`, so never
compare beats with `===`.

| Module | What it gives you |
|---|---|
| `tempoMap.ts` | `TempoMap` (beats ↔ seconds, bars/beats), `useTempoMap()` |
| `viewport.ts` | pure beats/seconds ↔ px mapping and zoom math |
| `viewStore.ts` | per-view zoom/scroll store: `createTimelineViewStore`, `useViewport`, `useTimelineView` |
| `grid.ts` | grid settings, adaptive resolution, grid lines, snapping |
| `selection.ts` | selection of clips, notes and automation points (multi-select), plus a time range |
| `marquee.ts` | marquee geometry and the `useMarquee` pointer hook |
| `Ruler.tsx` | ruler component: ticks, labels, loop brace and playhead marker |
| `PlayheadLine.tsx`, `playhead.ts` | full-height playhead line, playhead positioning and follow hooks |
| `useTimelineWheel.ts` | wheel/trackpad zoom and scroll |
| `format.ts` | `formatBarBeat` ("1.1.1"), `formatDuration`, `formatSeconds` ("m:ss.mmm") |
| `rulerMarks.ts`, `loop.ts` | pure ruler layout and loop-drag math, for canvas rulers and tests |

## Tempo map

```ts
const tempo = useTempoMap();           // memoized on the project's tempo/signature tables
tempo.beatsToSeconds(8); tempo.secondsToBeats(4);
tempo.bpmAt(b); tempo.signatureAt(b);
tempo.barBeat(5.5);                    // { bar: 2, beat: 2, fraction: 0.5 } (1-based)
tempo.barAt(b); tempo.nextBar(line); tempo.barToBeats(3); tempo.barLines(from, to, everyBars);
TempoMap.constant(120) / TempoMap.fromProject(project) / new TempoMap(points, signatures)
```

The semantics mirror Rust `ether_model::TempoMap`, which is the source of truth:

- Step and Linear curves (a Linear curve ramps BPM over beats).
- Seconds are counted from beat 0.
- Tempo is constant before the first point and after the last one.
- A signature change that falls off a bar line ends the previous bar early.

`tempoVectors.test.ts` checks the TS implementation against
`crates/ether-model/tests/tempo_vectors.json`. The test is skipped until that file lands
on main.

## Viewport and zoom/scroll store

Each view creates its own store. Views that scroll together share one store: the
arrangement ruler and its lanes, for example.

```ts
const view = useMemo(() => createTimelineViewStore({ pxPerBeat: 20 }), []);
const vp = useViewport(view);                 // { pxPerBeat, scrollBeats }, re-renders on change
const x = beatsToPx(clip.start, vp);          // content-relative px
const b = pxToBeats(e.clientX - rect.left, vp);
view.getState().zoomBy(1.25, anchorPx);       // zoom around the cursor
view.getState().scrollByPx(dx); view.getState().scrollTo(beats);
view.getState().zoomToRange({ start, end }, 16); view.getState().reveal(beats, 40);
view.getState().setWidth(px);                 // the Ruler does this unless syncWidth={false}
```

Store state: `pxPerBeat`, `scrollBeats`, `widthPx`, `limits` (zoom clamps, `scrollBeats >= 0`)
and `followPlayhead`. The pure functions (`zoomBy`, `zoomTo`, `zoomToRange`, `revealBeats`,
`clampViewport`, `visibleRange`, `secondsToPx`, `pxToSeconds`) work without a store.

Canvas code should read `view.getState()` and `view.subscribe(...)` instead of re-rendering.

## Grid and snapping

```ts
type GridSetting =
  | { type: "Adaptive"; density: "narrowest" | "narrow" | "medium" | "wide" | "widest"; triplet: boolean }
  | { type: "Fixed"; step: GridStep; triplet: boolean }
  | { type: "Off" };
type GridStep = { kind: "beats"; beats: number } | { kind: "bars"; bars: number };

const step = resolveGrid(setting, vp.pxPerBeat, tempo.signatureAt(vp.scrollBeats)); // null = off
gridLines(tempo, visibleRange(vp, width), step)   // [{ beats, level: "bar" | "beat" | "sub", bar? }]
snapToGrid(beats, step, tempo, "nearest" | "floor" | "ceil")
snapDelta(origin, delta, step, tempo, relative?)  // drags: absolute or keep off-grid offset
formatGridStep(step)                              // "1/16", "1/8T", "1 Bar", "4 Bars", "Off"
```

- The adaptive grid picks the finest step whose lines are at least `DENSITY_MIN_PX[density]`
  px apart. It uses 1/64 note up to a half note inside a bar, then 1, 2, 4... bars.
- Sub-bar steps count from each bar line, so 7/8 and signature changes work. The bar end
  is also a snap target.
- Holding alt to bypass snapping is up to the caller. Pass `step = null` in that case.

## Selection (clips, notes, automation points)

The app-wide **selected track** is not here. It lives in `@/state/selection`
(`useSelectionStore`). If a click on a clip should also focus its track, call
`useSelectionStore.getState().selectTrack(track)` yourself.

```ts
itemSelection                                    // app-wide store, pruned against the project mirror
const ids = useSelectedItems("note");            // ReadonlySet<NoteId>
const on = useIsSelected("clip", clip.id);       // re-renders only when this answer changes
itemSelection.getState().select("clip", [id], selectModeFromEvent(e)); // replace | add (shift) | toggle (cmd/ctrl)
itemSelection.getState().select("note", ids, "remove");
itemSelection.getState().clear("note");          // clear() with no argument clears everything incl. the time range
itemSelection.getState().setTimeRange({ start, end }); useTimeRangeSelection();
itemSelection.getState().anchor.note             // last clicked, for shift-range selection
```

- Kinds are independent sets. Selecting notes doesn't touch the clip selection.
- Sets are replaced on change, never mutated, so they are safe as selector results.
- Removed entities are deselected automatically.
- `createItemSelectionStore()` plus `bindSelectionToProject()` give you an isolated store
  for tests or a separate editor.

## Marquee

```tsx
const marquee = useMarquee({
  kind: "note",
  hitTest: (rect) => marqueeHits(rect, notes.map((n) => ({ id: n.id, rect: noteRect(n) }))),
});
<div onPointerDown={marquee.onPointerDown}>
  {marquee.rect && <div className="my-marquee" style={{ left: marquee.rect.x0, ... }} />}
</div>
```

- `rect` is in the element's local px. A drag replaces the selection with the hits,
  shift-drag adds, and cmd/ctrl-drag toggles, all relative to the selection at drag start.
- A click with no drag on the background clears that kind's selection.
- Attach the hook to the background only. Items should `stopPropagation` in their own
  pointer handlers.

## Ruler and playhead

```tsx
<Ruler view={view} />                                   // arrangement: bars, loop brace, click to locate
<Ruler view={view} format="seconds" />
<Ruler view={view} showLoop={false} tempo={clipTempo}
       playheadMapping={(song) => song - clipStart} onLocate={...} />  // clip-relative views
<div style={{ position: "relative" }}><PlayheadLine view={view} /> ...lanes... </div>
```

Ruler interactions:

- **Click** locates the playhead with `Transport::Locate`, snapped to the grid. Hold alt to
  bypass snapping.
- **Drag** scrolls horizontally and zooms vertically (drag down to zoom in).
- **Cmd/ctrl + wheel** zooms. A horizontal wheel scrolls.
- **Loop brace:** drag the body to move it and the edges to resize it. Shift-drag on the
  ruler draws a new loop, and double-click toggles it. Each loop drag is sent with a
  gesture and ends with `Edit::EndGesture`, so it is one undo step.

The ruler reads `project.settings.loop_region` / `loop_enabled` from the mirror. It also
drives `useFollowPlayhead(view)`: while playing with `followPlayhead` on, it pages the view
when the playhead leaves it.

The playhead never re-renders React. `usePlayheadPosition(ref, view, mapping?)` writes a
`transform` from `playheadStore`. `PlayheadLine` is the ready-made full-height line.
`useTimelineWheel(ref, view)` adds the same wheel behavior to lanes.

Styles are in `timeline.css` and use kit tokens. Pass `className` to override them.
