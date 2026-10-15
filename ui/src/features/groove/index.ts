// The clip inspector mounts `ProjectSwingFields` (its "Groove" row); `GrooveControls` is
// mounted by the piano-roll toolbar. `GroovePanel` is the full panel (no tab mounts it now).

export { GroovePanel, ProjectSwingFields } from "./GroovePanel";
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
