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
