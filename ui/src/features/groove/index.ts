// OWNERSHIP: the `groove` node owns `ui/src/features/groove/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `GroovePanel` (detail tab "groove"): keep the export names and keep them prop-less (read state via hooks).
// `GrooveControls` is mounted by the piano-roll toolbar (shared touch).

export { GroovePanel } from "./GroovePanel";
export { GrooveControls, type GrooveControlsProps } from "./GrooveControls";
export {
  grooveMenuItems,
  grooveQuantizeCommand,
  humanizeCommand,
  setSwingCommand,
  useGrooveSettings,
  type HumanizeSettings,
  type QuantizeSettings,
} from "./grooveCommands";
