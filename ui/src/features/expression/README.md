# MIDI expression lanes

The lane area under the piano roll (`midi-expression`, v0.3). Contract: CONTRACTS.md §13.2,
Rust model `ether_model::expression`, commands `ether_protocol::expression`.

## Data model

- **Clip lanes** (`project.expression_lanes`): one channel-wide curve per `(clip, kind)`.
  `kind` is `Cc { controller 0..=119 }`, `PitchBend` or `ChannelPressure`. Point times are
  **content-relative beats**, like notes, so a lane lines up with the note grid and loops
  and moves with its clip.
- **Note expressions** (`project.note_expressions`): one per-note curve per `(note, kind)`,
  where `kind` is `Pitch`, `Pressure` or `Timbre`. Point times are **beats from the note
  start**. Points past the note end are kept but not played.
- A curve is **one value** (`points`, sorted by time). That makes it the unit of editing and
  of collab last-writer-wins. Values: CC, pressure and timbre are 0..1, bend is -1..1, note
  pitch is ±96 semitones. Segments use the automation formula (`Linear`, `Step`,
  `Curve { tension }` = `x^(4^tension)`, see `@/features/automation/curve`). The first and
  last values hold outside the points.

`model.ts` holds the pure rules (ranges, `checkPoints`, `replaceRange`, stroke thinning,
labels). The MockTransport (`transport/mock/roadmap/expression.ts`) uses them, so mock and
engine validate the same way.

## Editing

| Where | Gesture | Command(s), all in one undo gesture |
|---|---|---|
| Clip lane | drag on the background (pencil) | `ReplaceRange` per pointer move; the first stroke on a missing lane is wrapped in a `Batch` with `CreateLane` |
| Clip lane | drag a point (grid snap, alt = free) | `SetPoints` |
| Clip lane | double-click a point / the background | `SetPoints` (delete / add) |
| Clip lane | right-click, or the "Remove lane" button | `SetPoints []` (clear), `RemoveLane` |
| Note lane | drag (pencil) over the selected notes (all notes when none is selected) | `Batch` of `SetNoteExpression` per touched note (the stroke range of each note's curve is replaced) |
| Note lane | right-click | `ClearNoteExpressions` |

The lane picker (`ExpressionLanes.tsx`, choices in `choices.ts`) offers Velocity, the
per-note kinds (`NOTE_KINDS`), the common controllers (`COMMON_KINDS`: pitch bend, channel
pressure, CC 1, 11, 64), every other CC lane the clip has, and "Other CC…" (any 0..119).
Kinds whose lane holds points are marked "•".

## For `mpe`

- Per-note **Pitch** and **Timbre** curves: append them to `NOTE_KINDS`.
  `NoteExpressionLaneView` already takes any `NoteExpressionKind` and its range
  (`noteExpressionRange`: Pitch is ±96 semitones, so you may want a narrower view window,
  such as the track's `MpeSettings::note_pitch_range`).
- Per-note curves drawn *inside* the note rectangles of the grid can reuse `curvePath`
  with `toX = t => beatsToPx(note.start + t, vp)`.
- The mock has the copy and cascade helpers (`copyNoteExpressions`,
  `deleteNoteExpressions`, `copyClipExpression`, `deleteClipExpression`) for the document
  reducer.
