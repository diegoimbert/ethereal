/** What counts as translatable text (shared by the source scan and the pseudo-locale test). */

/**
 * Terms written the same in every language (formats, protocols, units): a literal made only
 * of these and numbers ("vst3", "48 khz") isn't translatable text. Not a casing exception:
 * app text has none, these are lowercase like every word (#524, owner: "lowercase everything").
 */
export const SAME_IN_EVERY_LANGUAGE = new Set(
  "aax aiff api asio au clap cors daw dc fm gr hpf lfo pwm rms vca flac gpu id ip ir irs jack lufs mcp midi mp3 mpe ogg osc url usb vst vst2 vst3 wav wasapi dbfs macos ios".split(
    " ",
  ),
);

/** Does `s` read as words (not a number, symbol, unit, acronym or message key)? */
export function isText(s: string): boolean {
  const trimmed = s.trim();
  if (!/\p{L}{2,}/u.test(trimmed)) return false;
  // A message key (`{ label: "settings.tabs.general" }`).
  if (/^[a-z0-9_]+(\.[a-z0-9_]+)+$/.test(trimmed)) return false;
  const words = trimmed.match(/[\p{L}\p{N}]+/gu) ?? [];
  return !words.every((w) => SAME_IN_EVERY_LANGUAGE.has(w.toLowerCase()) || /^\d/.test(w) || /^(ms|s|db|hz|khz|px|x|bpm|cpu)$/i.test(w));
}
