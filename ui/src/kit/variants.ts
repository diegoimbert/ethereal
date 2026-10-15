/** Shared variant vocabulary for kit components (see docs/DESIGN-SYSTEM.md). */
export type Size = "sm" | "md" | "lg";
export type Tone = "default" | "accent" | "danger" | "ghost";
export type StatusTone = "default" | "accent" | "ok" | "warn" | "danger";

export const SIZES: readonly Size[] = ["sm", "md", "lg"];
export const TONES: readonly Tone[] = ["default", "accent", "danger", "ghost"];
export const STATUS_TONES: readonly StatusTone[] = ["default", "accent", "ok", "warn", "danger"];

/** Resolves a Button tone, honoring the deprecated `variant` prop ("primary" = "accent"). */
export function resolveTone(tone: Tone | undefined, variant?: "default" | "primary" | "ghost"): Tone {
  if (tone) return tone;
  if (variant === "primary") return "accent";
  return variant ?? "default";
}
