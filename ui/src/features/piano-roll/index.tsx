// The editor drawer (`app/shell/tabs.tsx`) mounts `PianoRoll`: keep it prop-less (read state via hooks).
//
// `PianoRoll` edits the clip open in the detail editor: `useEditorStore.getState().openClip(id)`
// from `@/state` (the arrangement does this on double-click of a MIDI clip).

export { PianoRoll, PianoRollEditor, type PianoRollEditorProps } from "./PianoRoll";
/** The editor tab's name ("Beat editor" on a Drum Rack track's clip). */
export { useClipEditorLabel } from "./BeatGrid";
