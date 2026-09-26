// OWNERSHIP: the `ui-automation` node owns `ui/src/features/automation/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `AutomationLanes`: keep this export name and keep it prop-less (read state via hooks).
//
// For the arrangement: mount `<TrackAutomationLanes trackId view headerWidth grid />` in
// each track row's automation slot and feed `useAutomationHeight()` to `layoutRows`.
// See README.md.

export { AutomationLanes } from "./AutomationLanes";
export { AutomationLaneView, type AutomationLaneViewProps } from "./AutomationLaneView";
export { TrackAutomationLanes, type TrackAutomationLanesProps } from "./TrackAutomationLanes";
export { clampTension, curveFraction, evaluatePoints, shapeFraction, type CurvePoint } from "./curve";
export { PAN_INFO, VOLUME_INFO, targetKey, trackTargets, useTrackTargets, type TargetInfo } from "./params";
export {
  AUTOMATION_BAR_HEIGHT,
  LANE_HEIGHT,
  automationHeight,
  automationLaneHeights,
  resetAutomationUi,
  useAutomationHeight,
  useAutomationUi,
} from "./uiStore";
