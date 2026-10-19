# Piano roll scales

Set the global scale in **Projects → Project scale**, or edit the active scale in
the piano-roll toolbar's **Scale…** popover (the button shows the active scale, e.g.
"C Minor…"). Tracks default to **Follow Project Scale**. Use the **Track** select for
**Custom Track Scale** or **Chromatic / None**. The Project / Track label next to the
root and type selects tells you which setting they edit.

**Highlight** distinguishes scale tones and roots, and dims other pitches.
**Scale notes only** packs the matching rows together (the Scale… button shows as
pressed). Hidden notes are preserved and still play; select-all takes only the shown
notes; turn the toggle off to draw any pitch. Neither control quantizes notes or
restricts MIDI playback, recording, or semitone keyboard nudges. A vertical drag
(move or cmd-drag copy) moves by displayed rows; when dragging several notes, their
semitone intervals are preserved.

Project and track settings use the Rust `MusicalScale` / `TrackScale` document types,
exported through `ui/src/generated`. Changes use undoable `Project.SetScale` and
`Track.SetScale` commands, persist in `.ether` files, and survive duplication.
Older files default to chromatic and project inheritance.

`ui/src/domain/scales.ts` owns the reusable pitch-class, scale-membership and
inheritance utilities. `getScaleNotes(root, kind)` returns pitch classes in degree
order; melodic minor uses its ascending form, and Blues is minor blues. Both major
and minor pentatonic are available.

`geometry.ts` supplies the descending visible pitch rows used by keys, notes,
hit testing, marquee selection, drag targets, and viewport anchoring. Scale
highlighting and row filtering are editor-local visual preferences.

# Clips extended to the left

Dragging a MIDI clip's left edge past its content start gives it a negative `offset` (a
leading gap, `ether_model::clip`). The piano roll and the beat editor show the whole clip:
their axis starts at `contentFloor(clip)` = `min(0, offset)` (`clipTime.ts`; `RollSpace.floor`),
and notes can be drawn, moved, resized, pasted and stretched there, down to the clip start
(notes may have negative content starts, they play like any other). Bar 1 stays at content 0,
so the extended part is bar 0, -1, ... on the editor's ruler.

# Multi-clip editing

Select several MIDI clips in the arrangement (shift/cmd-click, or the clip menu's
**Edit N Clips in Piano Roll**) and the piano roll edits them all at once, as if they were
one clip, like Ableton Live 11:

- The time axis is the song timeline. A strip under the ruler shows each clip's band (its
  name, color and loop region); the grid is shaded outside every clip. Notes take their
  clip's color (or its track's); the notes of clips other than the focused one are dimmed.
  Clicking a band or a note focuses its clip. The toolbar title, the scale, the expression
  and MPE lanes and collab presence follow the focused clip; the velocity lane shows every
  note.
- Selection, marquee, ⌘A, move, resize, transpose, nudges, cmd-drag duplicate, delete,
  quantize, humanize, velocity (Alt-drag and lane), stretch, copy / cut / paste / duplicate
  and the insert marker all act across clips. One gesture is one undo step, even across
  clips (`Edit.Batch` / gesture, replicated in collab like any edit).
- A note stays in its clip: a move is clamped to the clip's region, as in Ableton. Drawing
  puts the note in the clip under the pointer (the focused clip's track first) and focuses
  that clip; nothing is drawn outside a clip. A paste puts each note in the clip under it
  (its source clip's track first, then the focused track) and drops notes over no clip;
  clips never grow while several are edited.
- A looping clip's first pass is editable; its loop repetitions show the loop's notes as
  ghosts. Drawing in a repetition writes into the loop.

`multiClip.ts` holds the mapping (note `start` + `clip.start − clip.offset`) and the
transport wrapper that maps the editor's commands back to clip content; `rollSpace.ts`
describes the axis (one clip or several) to the editor's components.

# Drawing (canvas)

The note grid's rows, grid lines, loop ghosts and notes, and the velocity lane's bars, are
drawn on canvases (WebGL2 instanced rounded rects, a 2D-context fallback), not as DOM
elements: thousands of notes made zooming and scrolling lag.

- `noteLayout.ts` (pure, unit-tested): the theme palette (the former CSS `color-mix`es),
  the layers as instance lists culled to the visible part, `hitNote` / `hitVelocityBar`
  (topmost note under a point and its edge/body zone) and `gridCursor`.
- `noteCanvas.ts`: `InstanceList` and `RectRenderer` (fill, solid or dashed border with its
  own top edge, outer glow for the selected notes; one draw call per layer).
- `NoteGrid.tsx`: a rows canvas under the few DOM overlays (shaded outside, clip starts,
  section, marker) and a notes canvas over them; both cover the scroller's visible rows plus
  a margin and follow the vertical scroll. A render redraws at once; scroll and selection
  changes once per frame. Pointer events land on the grid and are hit-tested.
- Tests read what is drawn with `gridScene(grid)` / `velocityScene(lane)`; end-to-end tests
  with the grid element's `ethScene()` (`apps/web/e2e/ui.ts`: `pianoRollNotes`).
