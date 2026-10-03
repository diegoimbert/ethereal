/** MIDI expression lanes (v0.3, `midi-expression`): see README.md. */
export { ExpressionLanes, type ExpressionLanesProps } from "./ExpressionLanes";
export { choiceKey, COMMON_KINDS, NOTE_KINDS, type LaneChoice } from "./choices";
export { ExpressionLaneView, type ExpressionLaneViewProps } from "./ExpressionLaneView";
export { NoteExpressionLaneView, type NoteExpressionLaneViewProps } from "./NoteExpressionLaneView";
export { useExpressionLanesOf, useNoteExpressions } from "./hooks";
export * from "./model";
