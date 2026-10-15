import { t } from "@/i18n";

/** The buffer sizes (frames) offered, within the output's range. */
export const BUFFER_SIZES = [32, 64, 128, 256, 512, 1024, 2048];

/**
 * The buffer size select: the size the device runs (what the callbacks get, from the
 * status) rather than the one asked for, so a size the device can't run (clamped into its
 * range) or a driver that ignores the request (WASAPI shared mode) shows as it is. Offers the
 * sizes within the output's range.
 */
export function bufferChoice(
  requested: number | null,
  running: number | undefined,
  range: readonly [number, number] | null,
  settable: boolean,
): { value: number | null; options: number[]; note: string | null } {
  const actual = running && running > 0 ? running : null;
  const inRange = BUFFER_SIZES.filter((b) => !range || (b >= range[0] && b <= range[1]));
  const value = actual ?? requested;
  const options = [...new Set([...inRange, ...(value !== null ? [value] : [])])].sort((a, b) => a - b);
  let note: string | null = null;
  if (!settable) {
    note = actual ? t("settings.audio.buffer_wasapi_actual", { actual }) : t("settings.audio.buffer_wasapi");
  } else if (actual !== null && requested !== null && actual !== requested) {
    note = range
      ? t("settings.audio.buffer_mismatch_range", { actual, requested, min: range[0], max: range[1] })
      : t("settings.audio.buffer_mismatch", { actual, requested });
  }
  return { value, options, note };
}
