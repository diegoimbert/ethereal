/** MIDI expression lanes (v0.3): see README.md. */
export { ExpressionLanes, type ExpressionLanesProps } from "./ExpressionLanes";
export { ClipControllerLanes, CONTROLLER_LANE_HEIGHT, EditedClipControllerLanes, type ClipControllerLanesProps } from "./ClipControllerLanes";
export { compareKinds, controllerBadgeText } from "./badge";
export { choiceKey, COMMON_KINDS, NOTE_KINDS, type LaneChoice } from "./choices";
export { ExpressionLaneView, type ExpressionLaneViewProps } from "./ExpressionLaneView";
export { NoteExpressionLaneView, type NoteExpressionLaneViewProps } from "./NoteExpressionLaneView";
export { useExpressionLanesOf, useNoteExpressions } from "./hooks";
export * from "@/domain/expression";
