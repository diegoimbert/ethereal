import { formatDate } from "@/i18n";

/** Longest checkpoint name the engine keeps. */
export const MAX_CHECKPOINT_CHARS = 120;

/** Clock time of a step ("14:03:12"); the full date is in the row's tooltip. */
export function stepTime(ms: number): string {
  return formatDate(ms, { hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23" });
}
