// OWNERSHIP: the `ui-piano-roll` node owns `ui/src/features/piano-roll/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `PianoRoll`: keep this export name and keep it prop-less (read state via hooks).
//
// `PianoRoll` edits the clip open in the detail editor: `useEditorStore.getState().openClip(id)`
// from `@/state` (the arrangement does this on double-click of a MIDI clip).

export { PianoRoll, PianoRollEditor, type PianoRollEditorProps } from "./PianoRoll";
