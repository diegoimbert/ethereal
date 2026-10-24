# `ui/src/features/automation`: automation lanes (SVG)

Breakpoint automation lanes drawn in SVG, for track lanes in the arrangement and clip
envelopes in the clip editors. Import from `@/features/automation`.

| Module | What it is |
|---|---|
| `TrackAutomationLanes.tsx` | the per-track component the arrangement mounts under each track row |
| `AutomationLaneView.tsx` | one SVG lane (curve + points + editing), reusable for clip envelopes |
| `AutomationLanes.tsx` | the detail-view tab (prop-less, mounted by the app shell): selected track's lanes with their own ruler |
| `curve.ts` | `curveFraction` / `evaluatePoints`: exact mirror of `ether_core::automation` |
| `geometry.ts` | value ↔ y, time ↔ x, lane SVG path, point hit boxes (pure) |
| `baseline.ts` | a target's current (document) value: the empty lane's dashed baseline and the first point's value |
| `edit.ts` | protocol commands for add / move / delete / curve / bend (pure) |
| `gesture.ts` | `LaneGesture`: one gesture id per drag, `Edit::EndGesture` on release |
| `params.ts` | automatable targets of a track and their `ParamInfo` |
| `uiStore.ts` | open tracks, shown lanes per track, lane heights and value windows, total heights |
| `laneMotion.ts` | open/close tween of the slot heights (`useAutomationSlotHeight`, `animatedSlotHeight`, `subscribeLaneFrames`) |
| `valueAxis.ts` | param-aware value axis: steps, snapping, increments, visible window, step lines (pure) |
| `ValueScale.tsx` | lane header value scale and resize grip |
| `clipboard.ts` | point and range copy / paste (remap, one undo step) |
| `range.ts` | time-range edits of one lane: slice a range, write over one keeping the curve outside (pure) |
| `ClipEnvelopes.tsx` | clip envelope lane, its on/off and delete buttons, the warp editor's envelope strip |
| `clipEnvelopeTargets.ts` | a clip's envelope lanes and the envelope picker options |

## Clip envelopes

A clip envelope is an automation lane owned by a clip (`owner: { type: "Clip" }`), like
Ableton's clip envelopes. Its point times are **content-relative beats** (like notes), so it
moves with the clip, loops with the clip loop and is copied with the clip. Targets are the
track's (volume, pan, sends, device on/off and device params) and, for audio clips, the
clip's own **Clip Gain** and **Transposition** (`AutomationTarget::ClipGain` /
`ClipTranspose`, group "Clip", listed first).

**Precedence** (engine, `ether_core::automation_rt`; CONTRACTS.md §4): while an unmuted clip
plays, its enabled envelope **overrides** the track's arrangement lane for that target (the
last active clip wins when clips overlap); elsewhere the arrangement lane applies. A disabled
envelope ("Off") is kept and ignored, so the track's automation plays instead. This is
Ableton's arrangement behaviour for clip envelopes that hold absolute values.

Clip Gain and Transposition have no track lane to override: they **modulate** the clip's
own value, like Live's sample envelopes. Clip Gain adds its dB (volume fader law, the bottom
is silence) to the clip gain; Transposition adds its semitones (±48, total clamped to ±48)
to the clip transpose. Complex (stretched) clips change pitch only; Repitch and unwarped
clips change speed too, and the engine integrates the varying rate so positions stay exact
after a locate or loop (`ether_core::warp::RateIntegral`).

Where to edit them:

- **Piano roll**: the lane picker under the notes lists every parameter in groups
  "Envelope · Mixer" / "Envelope · <device>"; parameters whose envelope holds points come
  first ("Envelopes in this clip", marked "•"). The lane is drawn on the piano roll's content
  axis and snaps to its grid.
- **Warp editor** (audio clips): the envelope strip under the waveform, on the waveform's axis.

Editing is the track lane's (`AutomationLaneView`, every gesture in the table below). The
lane is created with its first point; "Off" disables it, "Delete envelope" removes it with
its points. Paste at the edit cursor pastes at the content position playing there.

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
| Bar | click / drag its empty space | like the track's empty clip-lane space: select the track, insert marker (and playhead when stopped); drag: select clips, `⇧`-drag: time section |
| Lane header | parameter menu / `×` / `⏻` / curve menu / `⌫` | switch parameter / hide lane / enable / curve of selected points / delete lane |
| Lane header, bottom edge | drag | resize the lane in 8 px steps (`⌥`: free); double-click: reset. UI state, like track heights |
| Value scale (right of the lane header) | wheel (when zoomed in) / drag | scroll the visible value window (at full range the wheel scrolls the arrangement) |
| Value scale | `⌘`-wheel | zoom the value window around the pointer |
| Value scale | double-click | reset the window |
| Lane (track lanes) | click empty space | select the track and the lane (focus), clear the other selections, place the insert marker at the click (snapped, `⌥`: free) and, when stopped, the playhead: like a click on the track's empty clip-lane space |
| Lane | drag empty space | like the arranger: marquee-select points (`⌘`: toggle them); a click places the insert marker (track lanes) |
| Lane (track lanes) | drag empty space past the lane | the box goes on across the other lanes and tracks (`laneMarquee.ts`, `laneRegistry.ts`): it selects the points of every track lane it crosses and the clips of the clip lanes it crosses; a box started in a clip lane takes the points of the lanes it crosses too |
| Several lanes selected | drag a point / arrows / `⌫` / `⌘C` `⌘X` `⌘V` `⌘D` | act on every lane's selected points as one undo step: a drag moves them alike in time, each lane turns the vertical motion into its own values; a paste puts each lane's points back into its own lane, in step |
| Clips and points selected | `⌫` / `⌘C` `⌘X` `⌘V` `⌘D`, from a lane or the clips | act on both, one undo step (`arrangement/mixedSelection.ts`); dragging a clip moves the points along in time, dragging a point moves the clips along (they keep their tracks) |
| Lane (track lanes) | `⇧`-drag empty space | select a time range in this lane (snapped, `⌥`: free); `⇧`-click extends the range (or the track's insert marker) to the click |
| Lane (track lanes), with a time section | `⇧`-drag the line inside the section (within 6 px of it) | move the section up/down; the lane outside it doesn't change: each edge becomes a jump (two points at the same time, outer one kept, inner one moving), added once the pointer moved 3 px, reusing points already there (`sectionMovePlan` in `range.ts`). The inside moves like a vertical multi-point drag (stepped params by steps, the group clamped to the range, `⌥`: precision). A `⇧`-click there changes nothing. The section is the lane's own or the track row's. One undo step |
| Lane | double-click empty space | add a point on the SECOND PRESS (not the release) at the clicked time (on the grid, `⌥`: free) with the lane's value there, never the pointer's y, so adding a point never changes what plays (`splitPointAt` in `curve.ts`: the engine's evaluation, step/linear exact, a curve split keeps its left half exactly and fits the right half's tension, within ~2 % at extreme tensions; an empty lane: the target's current value). Moving more than 3 px (touch: 8 px) while held drags it exactly like a grabbed point (same code: grid, steps, `⇧`/`⌥`/`⌘`, a point landing on a neighbour's time keeps its side), relative to where it was added; a release before that changes nothing. One undo step |
| Lane | click / `⇧`-click / `⌘`-click a point | select / add / toggle |
| Lane | drag a point | move the selection: time on the grid; stepped params (semitones, enums, toggles) by whole steps, 8 px per step |
| Lane, dragging | `⇧` held | continuous params: whole increments of the displayed value (1 dB, 1 %, 1 st, 1 ct, 0.01 pan, 0.1 ratio; Hz/ms on two significant digits: 1 Hz below 100 Hz, 10 Hz to 1 kHz, 100 Hz above), so points land on round values. Stepped params always snap |
| Lane, dragging | `⌥` held | precision: time off the grid (the app-wide meaning) and the point moves 1/10 of the pointer's motion, counted from the press, so pressing or releasing it mid-drag never jumps (`precisionDrag`). With `⇧`, `⇧`'s increments win for the value; only the time is slowed. The tempo and expression lanes do the same |
| Lane, dragging | `⌘` held | lock to the dominant axis |
| Lane, dragging | — | a tooltip shows the value and these modifiers (from the keymap's `POINT_DRAG_GESTURES`) |
| Lane | drag the line (within 6 px of it, not on a point) | move that segment up/down: its two points' values, times kept (before the first / after the last point: that point), like Ableton and Bitwig; the two points become the selection. Value rules of a point drag (steps, `⇧` increments, `⌥` precision); a click without moving is a click on the lane. The cursor shows `ns-resize` over the line |
| Lane | `⌥`-drag a segment | bend it (curve tension) |
| Lane | double-click a point | delete it |
| Lane (focused) | `⌘C` / `⌘X` / `⌘V` / `⌘D` | copy / cut / paste at the selection cursor / duplicate right after the selection |
| Lane (focused), with a time range | `⌘C` / `⌘X` / `⌘V` / `⌘D` / `⌫` | copy / cut / paste (after the range) / duplicate (right after it; the copy becomes the range) / delete the automation of the range; the curve outside it is kept (see Range edits) |
| Lane (focused) | `⌫` / `⌘A` | delete selected / select all points of the lane |
| Lane (focused) | `↑` `↓` | nudge values one step (stepped) or 1 % (`⇧`: to the next whole increment, e.g. 1 dB; `⌥`: 0.1 %) |
| Lane (focused) | `←` `→` | nudge times one grid step (`⇧`: a quarter step) |
| Point | right-click | Cut, Copy, Paste, Duplicate, Set Value… (a popover at the point: type a value in its unit, Enter or a click outside applies, Esc cancels), Linear/Step/Smooth curve, Delete |
| Lane | right-click empty space | Paste Here, Paste at Cursor, Select All Points, Copy, Delete |
| Lane | right-click in its time range | Cut, Copy, Paste, Duplicate, Delete (of the range) |

### Range edits (`range.ts`)

A track lane's time range is the arrangement's time selection (`features/time-edits`) with
`automationLane` set to the lane's target key: one selection app-wide, so a click elsewhere
replaces it, but the track time edits (`TimeEdit::*`) ignore it (`trackRangeOf`). The lane
draws it (the arrangement's layer only draws whole-track ranges); the insert marker and the
playhead are drawn across the whole row, automation lanes included.

Range edits write over the range with `RemovePoints` + `AddPoints` in one `Edit::Batch` (one
undo step); no engine command was needed. Edge points keep the lane's value right before
and after the range (a jump where the two sides differ), like the engine's time edits
(`time_edit/points.rs`). Delete leaves a ramp between the two edge values; copy holds the
range's edge values, so a paste or duplicate reproduces the range exactly.

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
  Steps come from `ParamInfo.step` when present; enums (`labels`) and
  `Toggle` are stepped either way; only descriptors without the field (older descriptors)
  also treat linear `Semitones` as whole semitones.
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
(`paramToPlain` + `formatParam` from `domain/paramScale.ts`, the mirror of the
Rust `scale_to_plain`):

- device params: the engine's descriptor (`Device::GetDescriptor`), automatable and not
  hidden params only;
- track volume and send level: `Fader` law, `-144..+6 dB` (same as the track volume fader);
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
