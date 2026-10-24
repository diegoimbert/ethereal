// The editor drawer (`app/shell/tabs.tsx`) mounts `AutomationLanes`: keep it prop-less (read
// state via hooks).
//
// For the arrangement: mount `<TrackAutomationLanes trackId view headerWidth grid />` in
// each track row's automation slot and feed `useAutomationHeight()` to `layoutRows`.
// See README.md.

export { AutomationLanes } from "./AutomationLanes";
export { AutomationLaneView, type AutomationLaneViewProps } from "./AutomationLaneView";
export {
  ClipEnvelopeButtons,
  ClipEnvelopeLane,
  ClipEnvelopeStrip,
  NotesBackdrop,
  type ClipEnvelopeLaneProps,
  type ClipEnvelopeStripProps,
} from "./ClipEnvelopes";
export { ENVELOPE_PREFIX, clipEnvelopeOptions, envelopeKeyOf, useClipEnvelopes, useClipLanes } from "./clipEnvelopeTargets";
export {
  animateGroupFold,
  animatedSlotHeight,
  groupFoldProgress,
  isGroupFoldAnimating,
  subscribeLaneFrames,
  useAutomationSlotHeight,
  useLaneMotion,
} from "./laneMotion";
export { pointsInRect, registerClipHitTester } from "./laneRegistry";
export { TrackAutomationLanes, type TrackAutomationLanesProps } from "./TrackAutomationLanes";
export { useAutomationToggle } from "./toggle";
export { openValueMenu, ValueEditHost, valueMenuEntries, type ValueMenuTarget } from "./valueMenu";
export { AutomationToggleButton, type AutomationToggleButtonProps } from "./AutomationToggleButton";
export { clampTension, curveFraction, evaluatePoints, shapeFraction, type CurvePoint } from "./curve";
export { PAN_INFO, SEND_INFO, VOLUME_INFO, targetKey, targetNameIsUserText, trackTargets, useTrackTargets, type TargetInfo } from "./params";
export {
  AUTOMATION_BAR_HEIGHT,
  LANE_HEIGHT,
  automationHeight,
  automationLaneHeights,
  resetAutomationUi,
  useAutomationHeight,
  useAutomationUi,
} from "./uiStore";
