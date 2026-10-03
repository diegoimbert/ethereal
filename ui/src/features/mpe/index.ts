/**
 * MPE (v0.3, `mpe`; CONTRACTS.md §13.3): a MIDI track's MPE settings (inspector), the
 * per-note pitch curves drawn in the piano roll, and the shared settings model. Per-note
 * `Pitch` / `Pressure` / `Timbre` curves are edited in the expression lane area
 * (`@/features/expression`, `NOTE_KINDS`); the Pitch lane shows ±`pitchWindow(track.mpe)`.
 */
export { MpeSettingsFields, type MpeSettingsFieldsProps } from "./MpeSettingsFields";
export { NotePitchCurves, type NotePitchCurvesProps } from "./NotePitchCurves";
export * from "./model";
