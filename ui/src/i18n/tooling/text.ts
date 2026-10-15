/** What counts as translatable text (shared by the source scan and the pseudo-locale test). */

/**
 * Words that keep their capitals in every locale (the lowercase rule, #524) and are never
 * translated: acronyms, units, format names. Matched as whole words.
 */
export const PROPER_NOUNS = [
  "AAX",
  "AI",
  "IA",
  "AIFF",
  "API",
  "ASIO",
  "AU",
  "CLAP",
  "CORS",
  "DAW",
  "DC",
  "EQ",
  "FM",
  "GR",
  "HPF",
  "PWM",
  "RMS",
  "FLAC",
  "GPU",
  "Hz",
  "kHz",
  "ID",
  "IP",
  "IR",
  "IRs",
  "LFO",
  "VCA",
  "JACK",
  "LUFS",
  "MCP",
  "MIDI",
  "MP3",
  "MPE",
  "OGG",
  "OSC",
  "URL",
  "USB",
  "VST",
  "VST2",
  "VST3",
  "WAV",
  "WASAPI",
  "dB",
  "dBFS",
  "macOS",
  "iOS",
] as const;

const PROPER = new Set<string>(PROPER_NOUNS);

/** Does `s` read as words (not a number, symbol, unit, acronym or message key)? */
export function isText(s: string): boolean {
  const trimmed = s.trim();
  if (!/\p{L}{2,}/u.test(trimmed)) return false;
  // A message key (`{ label: "settings.tabs.general" }`).
  if (/^[a-z0-9_]+(\.[a-z0-9_]+)+$/.test(trimmed)) return false;
  const words = trimmed.match(/[\p{L}\p{N}]+/gu) ?? [];
  return !words.every((w) => PROPER.has(w) || /^\d/.test(w) || /^(ms|s|db|hz|khz|px|x|bpm|cpu)$/i.test(w));
}
