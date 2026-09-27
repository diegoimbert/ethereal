# `ui/src/features/automation`: automation lanes (SVG)

Breakpoint automation lanes drawn in SVG, for track lanes in the arrangement and, later,
clip envelopes. Import from `@/features/automation`.

| Module | What it is |
|---|---|
| `TrackAutomationLanes.tsx` | the per-track component the arrangement mounts under each track row |
| `AutomationLaneView.tsx` | one SVG lane (curve + points + editing), reusable for clip envelopes |
| `AutomationLanes.tsx` | the detail-view tab (prop-less, mounted by the app shell): selected track's lanes with their own ruler |
| `curve.ts` | `curveFraction` / `evaluatePoints`: exact mirror of `ether_core::automation` |
| `geometry.ts` | value ↔ y, time ↔ x, lane SVG path, point hit boxes (pure) |
| `edit.ts` | protocol commands for add / move / delete / curve / bend (pure) |
| `gesture.ts` | `LaneGesture`: one gesture id per drag, `Edit::EndGesture` on release |
| `params.ts` | automatable targets of a track and their `ParamInfo` |
| `uiStore.ts` | open tracks, shown lanes per track, lane heights and value windows, total heights |
| `laneMotion.ts` | open/close tween of the slot heights (`useAutomationSlotHeight`, `animatedSlotHeight`, `subscribeLaneFrames`) |
| `valueAxis.ts` | param-aware value axis: steps, snapping, increments, visible window, step lines (pure) |
| `ValueScale.tsx` | lane header value scale and resize grip |
| `clipboard.ts` | point copy / paste (remap, one undo step) |

## Mounting in the arrangement

The arrangement lays rows out from a model (`layoutRows` in `features/arrangement/layout.ts`),
not from the DOM, so it must get the automation heights from here. Two changes in
`ui/src/features/arrangement` (owned by ui-arrangement; to be done by alpha or a follow-up):

```tsx
// ArrangementView.tsx
import { useAutomationSlotHeight } from "@/features/automation";
const automationHeight = useAutomationSlotHeight();        // (track) => px; identity changes when heights change or a tween starts/ends
useLaneAnimation(contentRef, rows, rowsRef);                 // features/arrangement/laneAnimation.ts: per-frame heights
const rows = useMemo(() => layoutRows(tracks, folded, automationHeight), [tracks, folded, automationHeight]);

// TrackRow.tsx: replace the empty slot div
import { TrackAutomationLanes } from "@/features/automation";
const grid = useArrangementUi((s) => s.grid);
<div className="eth-arr-row__automation" data-slot="automation" data-track={row.track.id}>
  <TrackAutomationLanes trackId={row.track.id} view={arrangementView} headerWidth={HEADER_WIDTH} grid={grid} />
</div>
```

### `<TrackAutomationLanes>` props

| Prop | Type | Default | Meaning |
|---|---|---|---|
| `trackId` | `TrackId` | required | the track whose arrangement lanes (`owner: Track`) are shown |
| `view` | `TimelineViewStore` | required | horizontal viewport shared with the clip lanes (`arrangementView`) |
| `headerWidth` | `number` | `200` | width of the header column; lanes start after it (the arrangement's `HEADER_WIDTH`) |
| `grid` | `GridSetting` | `DEFAULT_GRID` | grid used to snap point times |
| `selection` | `ItemSelectionStore` | `itemSelection` | where selected points live (kind `automationPoint`) |

At rest its rendered height is `automationHeight(useAutomationUi.getState(), trackId)`:
`AUTOMATION_BAR_HEIGHT` plus each shown lane's height (`LANE_HEIGHT` unless resized) when
open; mid-animation it fills the slot it is given and clips its content. Non-React code can call `automationLaneHeights(trackId)`.

The component draws no playhead or grid lines: the arrangement's full-height overlays cover
it. It reads lane x positions from `view` (content-relative px, 0 = left edge of the lane
area), just like clips.

## Automation editing

Gestures and modifiers (for UX review). `⌘` is Ctrl off macOS.

| Where | Gesture | Does |
|---|---|---|
| Track header | automation icon | open/close the track's lanes (animated, see below) |
| Bar | `+ Parameter…` | show another parameter's lane (animates in) |
| Lane header | parameter menu / `×` / `⏻` / curve menu / `⌫` | switch parameter / hide lane / enable / curve of selected points / delete lane |
| Lane header, bottom edge | drag | resize the lane in 8 px steps (`⌥`: free); double-click: reset. UI state, like track heights |
| Value scale (right of the lane header) | wheel / drag | scroll the visible value window |
| Value scale | `⌘`-wheel | zoom the value window around the pointer |
| Value scale | double-click | reset the window |
| Lane | double-click empty space | add a point: time on the grid (`⌥`: free), value on the param's steps |
| Lane | click / `⇧`-click / `⌘`-click a point | select / add / toggle |
| Lane | drag a point | move the selection: time on the grid; stepped params (semitones, enums, toggles) by whole steps, 8 px per step |
| Lane, dragging | `⌥` | continuous params: whole increments (1 dB, 1 %, 1 st, 0.01 pan, round Hz/ms); time off the grid |
| Lane, dragging | `⇧` | lock to the dominant axis |
| Lane, dragging | — | a tooltip shows the value and these modifiers |
| Lane | drag empty space | marquee select |
| Lane | `⌥`-drag a segment | bend it (curve tension) |
| Lane | double-click a point | delete it |
| Lane (focused) | `⌘C` / `⌘X` / `⌘V` / `⌘D` | copy / cut / paste at the playhead / duplicate right after the selection |
| Lane (focused) | `⌫` / `⌘A` | delete selected / select all points of the lane |
| Lane (focused) | `↑` `↓` | nudge values one step (stepped) or 1 % (`⇧`: 0.1 %) |
| Lane (focused) | `←` `→` | nudge times one grid step (`⇧`: a quarter step) |
| Point | right-click | Cut, Copy, Paste at Playhead, Duplicate, Set Value…, Linear/Step/Smooth curve, Delete |
| Lane | right-click empty space | Paste Here, Paste at Playhead, Select All Points, Copy, Delete |

- **Paste** keeps relative timing, replaces the points in the pasted span, selects the
  pasted points and (when stopped) moves the playhead to their end, like clip paste. Into
  another parameter, values are remapped through the plain value when both share a unit
  (clamped to the target's range), else by normalized value; then snapped to the target's
  steps. Paste, cut and duplicate are each one undo step (`Edit::Batch`, creating the lane
  if needed). The desktop Edit menu's copy/cut/paste reach the focused lane too.
- **Stepped params** draw one gridline per step (every 2nd, 3rd, 4th, 6th, 12th... when
  dense), labelled on the value scale where they fit; the default value is stronger. They
  open on a window where every step is at least 8 px tall, around their default (a default
  64 px Transpose lane shows ±3 st; resize the lane or scroll/zoom the value scale for
  more), and dragging moves them by whole steps at 8 px per step, whatever the lane height.
  `ParamInfo` has no step field yet (BCR sent): steps come from `labels`, the `Toggle`
  unit, and linear `Semitones`.
- **Every drag is one `LaneGesture`**: all its `EditPoints` share one gesture id, closed
  with `Edit::EndGesture`, so it is one undo step. Moves are coalesced (latest wins while a
  command is in flight).

### Opening/closing animation (`laneMotion.ts`)

Opening/closing a track's lanes and showing/hiding a lane tween the automation slot height
over `motion.lane` (`duration.slow`, `ease.standard`; CSS: `--automation-lane-duration/ease`).
React lays the rows out once per tween, with the slot at the larger of its two heights
(`useAutomationSlotHeight`); each frame, the arrangement's `useLaneAnimation` then puts the
tweened heights into the row model used for hit testing (`rowsRef`, so clicks and drags
land where things are drawn) and moves what is on screen with transforms only: rows below
slide, and the lanes slide in/out of their clipped slot. Nothing re-renders per frame, so
64 tracks with 512 clips stay at 60 fps (see the e2e perf test). New content also fades in,
a closing track fades out, a hidden lane collapses in place. A change mid-animation
continues from the current height. Resizing a lane is direct (not animated). Reduced
motion (`MOTION.enabled`) is instant.

## Values

Point values are normalized 0..1. They are displayed through the target's `ParamInfo`
(`paramToPlain` + `formatParam` from `features/devices/paramScale.ts`, the mirror of the
Rust `scale_to_plain`):

- device params: the engine's descriptor (`Device::GetDescriptor`), automatable and not
  hidden params only;
- track volume and send level: `Fader` law, `-144..+6 dB` (same as the mixer);
- pan: `Linear`, `-1..1`.

## Curve formula and test vectors

`CurveShape::Curve { tension }` is drawn with the engine's formula, `x^(4^tension)` with
tension clamped to -1..1 (`ether_core::automation::curve_fraction`). Tension is an `f32`
in Rust, so the UI rounds it through `Math.fround` first. Step holds the value to the next
point; before the first and after the last point the value holds.

No Rust-side vector file existed, so `curveVectors.json` was generated from the Rust
functions themselves (a scratch binary depending on `ether-core` that prints
`curve_fraction(x, t)` for x in {0, .1, .25, .5, .75, .9, 1, -.5, 1.5} and t in
{0, .3, .5, 1, -.3, -.5, -1, 2, -2, .75, -.1}, plus `evaluate` over a mixed
Linear/Step/Curve lane). A few by hand: `curve_fraction(0.5, 1) = 0.0625`,
`curve_fraction(0.5, -0.5) = √0.5 ≈ 0.70711`, `curve_fraction(0.25, 0.5) = 0.0625`,
`curve_fraction(0.5, 2) = 0.0625` (clamped). `curve.test.ts` checks them to 1e-12. If
`ether-core` ever ships its own vector file, point the test at it instead.
