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
| `uiStore.ts` | open tracks, shown lanes per track, heights |

## Mounting in the arrangement

The arrangement lays rows out from a model (`layoutRows` in `features/arrangement/layout.ts`),
not from the DOM, so it must get the automation heights from here. Two changes in
`ui/src/features/arrangement` (owned by ui-arrangement; to be done by alpha or a follow-up):

```tsx
// ArrangementView.tsx
import { useAutomationHeight } from "@/features/automation";
const automationHeight = useAutomationHeight();            // (track) => px; identity changes when heights change
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

Its rendered height is always `automationHeight(useAutomationUi.getState(), trackId)`:
`AUTOMATION_BAR_HEIGHT` (the bar with the toggle, always present) plus `LANE_HEIGHT` per
shown lane when open. Non-React code can call `automationLaneHeights(trackId)`.

The component draws no playhead or grid lines: the arrangement's full-height overlays cover
it. It reads lane x positions from `view` (content-relative px, 0 = left edge of the lane
area), just like clips.

## Interactions

- **Bar:** `▸ Automation` opens/closes the track's lanes. The first open shows the track's
  existing lanes (or Volume). `+ Parameter…` shows another parameter's lane.
- **Lane header:** parameter chooser (switches what this row shows), `×` hides the row
  (the document lane stays), `⏻` enables/disables the lane (`SetLaneEnabled`), the curve
  menu sets Linear / Step / Curve on the selected points' segments, `⌫` deletes the lane.
- **Lane:**
  - double-click empty space: add a point, time snapped (alt: no snap), value from y;
    the lane is created in the same undo step (`Edit::Batch`) if needed;
  - click a point: select (shift adds, cmd/ctrl toggles); drag: move the selection, the
    dragged point snaps and the others keep their offsets; the group is clamped to time
    ≥ 0 and values 0..1; alt: no snap; shift while dragging: lock to one axis;
  - double-click a point: delete it; Delete/Backspace: delete the selected points of the
    focused lane; cmd/ctrl+A: select all points of the lane (both `preventDefault`, so the
    global shortcuts don't fire);
  - drag empty space: marquee (from `@/timeline`);
  - alt-drag a segment up/down: bend it (`CurveShape::Curve { tension }`).
- Every drag is one `LaneGesture`: all its `EditPoints` share one gesture id, closed with
  `Edit::EndGesture`, so it is one undo step. Moves are coalesced (latest wins while a
  command is in flight).

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
