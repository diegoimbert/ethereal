# Arranger renderer

The arranger's clips drawn on one canvas, with WebGL2, or with its Canvas2D fallback where
WebGL2 doesn't work. No DOM node per clip: the canvas sits over the lanes, between the grid
(below) and the rows' lane DOM (above: drop hints, live recording, import placeholders, the
playhead overlay, …). The DOM keeps what is naturally DOM: the rename field, the
audio-to-MIDI status, the ruler, markers, the insert and time-selection markers, popovers.

## Backends

`ArrangerRenderer` builds instance data (the scene and the layers write `rect`, `text`,
`note`, `wave` and their own pipelines' instances) and hands each batch to a backend
(`backend.ts`) with the row it belongs to:

- **WebGL2** (`webglBackend.ts`): the instanced pipelines of `shaders.ts`. The default.
- **Canvas2D** (`canvas2d.ts`): reads the same instance data on the CPU and paints it with
  Canvas2D: rounded rect fills and inner strokes (clip boxes cut as per-corner radii, or a
  clip path), the glow as a shadow (`shadowBlur`, the same Gaussian as CSS `box-shadow`)
  outside the box, text sprites from the same atlas tinted per ink and drawn 1:1, note
  rects, peaks as one path per span (a point per css px), and the layers' `fade` and
  `shape` pipelines as paths. So every layer works on it with no code of its own; a
  pipeline it has no painter for is skipped with a console warning (add a painter there
  when you add a pipeline). Simpler where it doesn't show (Canvas2D's own antialiasing,
  dashes and hatching as strokes); a frame of 1k clips costs about 1 ms of script.
  Without a 2D context (jsdom) it draws nothing, but the renderer still runs: hit tests,
  gestures and the test API work in unit tests.

The choice is automatic (`backendChoice.ts`): Canvas2D when `webgl2Supported()` is false,
when the WebGL context can't be created twice in a row (the layer remounts on a new canvas
once), when a lost context isn't restored within 3 s, or after 3 losses in a session.
There is no user setting. A hidden debug override forces the fallback:
`localStorage["eth.arr.renderer"] = "canvas2d"` (read at startup), or
`window.__ether.setArrangerRenderer("canvas2d" | "auto")`.

| File               | What                                                                                                                    |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------- |
| `GlClipLayer.tsx`  | React glue: the canvas, store subscriptions, pointer hit-testing, rename field, aria-live, WebGL failure reporting      |
| `renderer.ts`      | `ArrangerRenderer`: dirty-flag frame loop, per-lane draw passes, culling, test hooks                                    |
| `backend.ts`       | `RenderBackend`: what draws the batches (`begin`, `draw`, `end`, `readPixels`)                                          |
| `webglBackend.ts`  | The WebGL2 backend (uniforms, pipelines, batches)                                                                       |
| `canvas2d.ts`      | The Canvas2D fallback backend (a painter per pipeline)                                                                  |
| `backendChoice.ts` | Which backend: WebGL2 support, failures, the debug override                                                             |
| `scene.ts`         | `LaneScene`: per-lane instance data built from the project mirror (incremental)                                         |
| `pipelines.ts`     | The instanced pipelines (`rect`, `text`, `note`, `wave`) and their instance writers                                     |
| `shaders.ts`       | GLSL of the pipelines                                                                                                   |
| `glContext.ts`     | Context wrapper, `Pipeline`, `Batch` (instance buffer + VAO; its CPU copy is what Canvas2D reads), context-loss restore |
| `glyphAtlas.ts`    | Text atlas: browser-rasterized text sprites, layout with ellipsis, icons, shelf packing with LRU eviction               |
| `textRaster.ts`    | The browser's text rasterizer (Canvas2D `fillText` in the DOM's font, features and gamma)                               |
| `waveTiles.ts`     | Peak tiles (`PeakCache`) as rows of an `RG32F` texture, LRU                                                             |
| `palette.ts`       | Colors from the CSS tokens and the clip color mixes of `arrangement.css`                                                |
| `hitTest.ts`       | Pure hit test with the part priorities (handles > title band > body > small clips)                                      |
| `layers.ts`        | The layers API (below)                                                                                                  |

## Coordinates: lane space

Every instance is positioned in **lane space**, so pan, zoom and lane resizes never rebuild
instance data:

- **x** = a beat and a px offset: `x = (beat - scrollBeats) * pxPerBeat + px`, with an
  optional minimum width (`XSpec { b0, b1, p0, p1, minW }`: the box spans
  `[x0, max(x0 + minW, x1)]`). Clip boxes are `{ b0: start, b1: end, minW: 1 }` (their true width; the hit area keeps 8 px).
- **y** = offsets in the row: `y = a + f * mainHeight` from the row's top
  (`YSpec { a0, f0, a1, f1 }`). The clip box is `{ a0: 2, a1: -2, f1: 1 }`, its title band
  `{ a0: 2, a1: 16 }`.

The row's top and main height are per-draw uniforms (`u_row`); the view is `u_view`. The
canvas covers the viewport plus `MARGIN_Y` and is moved with the content (a transform) at
each frame, so vertical scrolling never shows clips a frame behind the rows.

## Pipelines

- `rect` (`writeRect`): SDF rounded rectangles with fill, inner stroke (`strokeWidth`), CSS
  box-shadow-like blur (`blur`: draws the rect's shadow), clipped by a rounded clip box
  (`clip`) or the lane (`RectFlag.ClipLane`). `RectFlag.Snap` snaps edges to device pixels.
  `RectFlag.HandleStart/End` make the rect the clip box's edge handle.
- `text` (`writeGlyph`): text sprites and icons from the atlas, drawn 1:1 at device pixels,
  clipped horizontally to an `XSpec`. Lay text out with `atlas.layout(text, font, maxWidth,
ink)` (ellipsis like CSS; see Text below).
- `note` (`writeNote`): note rects in the clip body, placed like `noteRects` (`../clipDraw.ts`).
- `wave` (`writeWave`): waveform spans sampled from the peak tile texture.

`fx` (`Fx.Muted`, `Fx.Frozen`) applies the clip filters (saturate + opacity against the
background); `alpha` multiplies (ghosts 0.45, dragged 0.85).

## Draw order (per lane)

1. layers at `z: "under-clips"`;
2. the lane's clips in start order (culled to the view): box + title band, waveforms, notes,
   loop lines / joined-edge separator / rim / hovered handle, text;
3. raised clips (selected, dragged) the same way, above;
4. the layers' extra clip lanes (`ExtraLane`: take lanes, the comp), each the same way;
5. layers at `z: "over-clips"`, then `z: "top"`;

then each layer's `frame()` once, after all lanes (`f.gl` on WebGL, `f.ctx2d` on Canvas2D).

## Layers API (phase 2: fades, warp markers, comping, automation, group summaries, presence)

```ts
import { onGlRenderer } from "@/features/arrangement/gl/active";
import { RectFlag, writeRect } from "@/features/arrangement/gl/pipelines";
import type { ArrangerLayer } from "@/features/arrangement/gl/layers";

const fades: ArrangerLayer = {
  id: "clip-fades",
  z: "over-clips",
  // Rebuilt when one of these changes (===). Default: [lane.staticVersion, lane.view].
  deps: ({ lane, input }) => [lane.staticVersion, input.selected],
  build({ lane, input }, out) {
    for (const it of lane.items) {
      if (it.clip.content.type !== "Audio") continue;
      const fadeIn = it.clip.content.fade_in; // beats
      writeRect(out.rects, {
        x: { b0: it.bounds.start, b1: it.bounds.start + fadeIn },
        y: { a0: 16, a1: -2, f1: 1 },
        fill: [0, 0, 0, 0.2],
        clip: { x: { b0: it.bounds.start, b1: it.bounds.start + it.bounds.length, minW: 1 }, y: { a0: 2, a1: -2, f1: 1 }, radius: 6 },
      });
    }
  },
  // Lane px (0 = lane's left edge) and px from the row top; wins over the clips.
  hitTest(x, y, row) {
    return null; // or { clip, track: row.track.id, part: "layer", data: { handle: "fade-in" } }
  },
  onPointerDown(hit, e) {
    /* start the gesture; e is the native PointerEvent */
  },
  cursor: () => "ew-resize",
};

// Mount: add the layer while the renderer exists.
useEffect(() => onGlRenderer((r) => (r ? r.addLayer(fades) : undefined)), []);
```

Notes:

- `onGlRenderer(fn)` calls `fn` now and on every mount/unmount (of either backend); `addLayer` returns its
  removal. `renderer.layers.invalidate(layer)` rebuilds a layer's lanes; any store the layer
  reads that isn't in `deps` must invalidate it (or call `renderer.requestRender()` for a
  redraw without a rebuild).
- `ctx.lane.items` are the lane's clips with the drag preview applied (`LaneItem`: `clip`,
  `bounds`, `dragging`, `ghost`), sorted by start. `ctx.input` holds the project, selection,
  hover, palette and tempo map; `ctx.window` the settled zoom and the beat window to build
  view-dependent instances for (text).
- Text: `out.atlas.layout(text, TITLE_FONT, maxWidth, ink)` then `writeGlyph` per glyph of
  the run (one sprite, plus the ellipsis when cut), at `out.atlas.baseline(top, height,
font)` + `glyph.top`. Pass the color the text is drawn in as `ink`. Widths:
  `atlas.measure(text, font)` (no rasterization).
- For something that isn't per lane (e.g. presence cursors across the view), use `frame()`:
  it gets the GL context (`f.gl`) after the lanes are drawn on WebGL (bind your own
  program; `Pipeline` and `Batch` from `glContext.ts` do the boilerplate), or the 2D
  context (`f.ctx2d`, css px, the canvas's transform set) on the Canvas2D fallback. Handle
  both, or draw nothing on the one you don't. No layer uses `frame()` today; prefer
  instances (they work on both backends).
- Hit priority: layers are asked first (last added first); return `null` to let the clips'
  own hit test run (handles > title band > body pass-through > small clips).
- Own pipelines: list them in `pipelines` (one instance per renderer: a `Pipeline` belongs to
  one GL context) and write with `out.custom(p)`; they draw after the layer's rects, with the
  shared uniforms set (`SHADER_COMMON` / `FRAG_COMMON` in shaders.ts). Examples: the `fade`
  pipeline in `layers/fadePipeline.ts`, the `shape` pipeline in `layers/shapes.ts`
  (`writeSegment`: polylines and circles; `writeOutline`: solid or dashed rounded outlines).
  A new pipeline also needs a painter in `canvas2d.ts` (by its `name`; constants baked in
  its shaders go in `Pipeline.params`), or the fallback skips it.
- Hover: `input.hover` is set for every hit (body and layer hits included) with
  `data` when the hit's `data` is a string, so a layer can show things while a clip is
  hovered (`deps` should include it; it doesn't bump `staticVersion` by itself).
- `onDoubleClick` / `onContextMenu` get the double-clicks and right-clicks on the layer's hits.
- Layers draw after all of a lane's clips: an overlay of a clip that other clips may cover
  clips itself to the spans no clip drawn above covers (`layers/fadeModel.ts` `visibleSpans`).
- Layers draw in the **whole row** of every track kind (group, return and VCA rows too),
  including its take lanes and automation slot below the main lane: use absolute `a`
  offsets from the row top there (`row.laneHeight`, `row.takesHeight`, `row.height`), and
  put them in `deps` (they change per frame while lanes animate).
- **Extra clip lanes**: `lanes(ctx)` returns `ExtraLane`s (`key`, `top`, `height`,
  `source`, `alpha`): clip lanes besides the track's main one (take lanes, the comp's
  pieces), drawn like the main lane (same scene code) after its clips and before the
  `over-clips` layers. `source.items(input)` must return the same array while nothing
  changed, and the same `source` object frame after frame (the scene is kept by key while
  its source is the same). `source.elements`: no small-clip rules (they are drawn in full
  at any width).
- View-dependent data in a layer (culled polylines, sampled curves): `layers/viewWindow.ts`
  (`WindowTokens`) keeps one build window per key until the view leaves it.

## In-clip editing layers (`layers/`)

`mountClipEditLayers` (from `GlClipLayer`): `clipFades.ts` (fade shapes, crossfade tint and X,
fade/curve handles and their gestures, as `ClipFades`), `clipBadges.ts` ("REV", "Missing" and
the missing-media hatch; the scene leaves room for them before the name). Clip gain and warp
need no layer (the `wave` pipeline scales and maps the peaks; arrangement clips show no warp
markers). Test API: `handleRect(id, "fade-in" | "fade-out" | "fade-in-curve" |
"fade-out-curve")`, `hitTest` reports those parts, `clipDecor(id)`, `overInk(id)`.

## Title items (reserves)

A clip's title band lays out like a flex row: loop icon, items before the name
(badges), the name (ellipsized), items after it (statuses, editors). A layer that draws an
item in the band reserves its room so the name is laid out around it:

```ts
renderer.addTitleReserve({
  id: "clip-editors",
  side: "right", // "left": after the loop icon, before the name
  // px incl. its 2 px gap; 0 when this clip doesn't show the item
  width: (clip, atlas, input) => (editorsOf(clip.id).length ? chipWidth(atlas, clip) + 2 : 0),
});
renderer.invalidateTitles(); // when a width may have changed (store subscription)
```

Place the item with `titleLayout(clip, widthPx, atlas, input, reserves)` (scene.ts):
`leftStart` (left items), `nameStart` / `nameRoom`, `rightStart` (right items). In use:
"clip-badges" (left, REV / Missing) and "audio-to-midi-status" (right).

Text instances hold atlas positions: every layer is rebuilt when the atlas resets
(font loaded, device pixel ratio changed), whatever its `deps`.

## Lane layers (phase 2b, `layers/`)

| File                 | What                                                                                                                                                           |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lanes.ts`           | `mountLaneLayers(renderer)` (from `GlClipLayer`)                                                                                                               |
| `shapes.ts`          | The `shape` pipeline: SDF segments, circles, dashed outlines                                                                                                   |
| `automationLanes.ts` | Automation lanes: `AutomationLaneView` (`gl`) publishes each lane; the layer draws background, step lines, envelope and points where the DOM lays the lane out |
| `comping.ts`         | The comp's pieces (extra lane + outlines, click selects the region) and take lanes (extra lanes, regions, swipe)                                               |
| `groupSummary.ts`    | Group lane summaries                                                                                                                                           |
| `testApi.ts`         | `automationPoint`, `compPiece`, `takeRegion`, `swipePreview`, `groupSummaryBars` on `window.__ether.arrangement`                                               |

## Interaction

`GlClipLayer` hit-tests presses on the lanes in the capture phase: title band and small clips
start the move gesture (`onClipPointerDown`, copy with cmd/ctrl), edge handles the
resize gestures (extend-left included), bodies let the press through to the lane (locate,
marquee, MIDI insert), double-click opens the editor, right-click the clip menu
(`fullClipMenu`). Rename shows a DOM `<input>` over the title band.

## Tests

`window.__ether.arrangement` (`../testApi.ts`): `renderer` (the backend), `clipRect`,
`handleRect`, `nameRect`, `hitTest`, `visibleClipIds`, `expectedVisibleClipIds`, `bodyInk`,
`stats`, `loseContext` (WebGL), on both backends. The e2e helpers (`apps/web/e2e/clips.ts`)
aim through it. Playwright projects: `webgl` runs every spec on the default backend;
`canvas2d` runs the arranger specs again with the fallback forced (`ETHER_E2E_ARRANGER=canvas2d`
runs every spec on it). Specs comparing the backends (`webgl-*.spec.ts`: clips, text, lanes,
presence, clip editing) switch at runtime and run in the `webgl` project; the never-disappear
and perf specs stay on WebGL. `canvas2d.test.ts` checks what the fallback paints per
pipeline against a recording 2D context.

## Text

Arranger text never scales with the zoom, so it is drawn as the browser draws it: each run
(a name, a badge label, the editors chip) is rasterized whole with Canvas2D `fillText`
(`textRaster.ts`) and drawn 1:1 at device pixels (`texelFetch`, a quad of the sprite's
device size at a device pixel), its coverage blended in sRGB like the browser composites
text. The `webgl-arranger-text` e2e compares the names with DOM text the test lays over the
clip (a `.eth-clip__name` element in the name's box, `nameRect`, with `text-overflow:
ellipsis`; both themes, 1x and 2x, both backends: within 2/255). What makes them match:

- **Font and features**: the clip name's computed style (`.eth-clip__name`); the app's
  `font-feature-settings` / `font-variant-numeric` (Inter's `cv11`, `tnum`), which a canvas
  can't set, go through the UI font's `@font-face` rules registered again under an alias
  with the features as a descriptor.
- **Shaping**: `textRendering = "optimizeLegibility"` (Chrome's canvas otherwise shapes
  word by word, losing contextual forms across spaces); spaces collapse as with
  `white-space: nowrap`; a cut name is drawn as its text then "…" as a run of its own, as
  Chrome paints `text-overflow: ellipsis` (cut at a grapheme cluster, a space before the
  "…" kept).
- **Gamma**: Skia adjusts glyph coverage by the ink's luminance, so a sprite is rasterized
  in its ink reduced to 3 bits per channel (`canonicalInk`, part of the cache key) and keeps
  the alpha only; the shader tints it with the exact ink (and the clip's fx).
- **Placement**: baselines at whole css px (`atlas.baseline`), as Chrome paints text at any
  device pixel ratio.

The atlas (`R8`, 2048 wide, grown up to 2048 rows as they fill) packs sprites on shelves
by height and uploads the rows changed.
Only new strings are rasterized. When it is full, shelves not used since the text was last
rebuilt are evicted (LRU); when all are in use, `version` is bumped once so the text is
rebuilt and marks what it still uses. `reset()` (fonts loaded, dpr changed) drops it all.
