// eth-allow-case-file: the rules themselves (the term lists).
// The UI's own text is lowercase: menus, buttons, dialogs, tooltips, toasts, labels,
// placeholders, empty states, aria-labels, readouts. No exceptions for words (owner: "lowercase
// everything"): acronyms, formats and units too ("midi", "vst3", "lfo", "-6.0 db", "440 hz"),
// product and platform names ("ableton live", "macos"). What isn't a word keeps its case: key
// caps in a shortcut ("⌘K", "Ctrl+Z"), note names ("C#4", "A minor"), pan and grid notation
// ("25L", "1/8T"). User content (names the user typed, plugin names and values a third-party
// plugin reports) is never rewritten. `uiCaseScan.ts` checks the source literals against these
// rules (`uiCase.test.ts`).

/** Notation that keeps its capitals: pan and grid ("C", "25L", "100R", "1/8T"). */
export const ALLOWED_WORDS = new Set(["C", "L", "R", "T"]);

/** Key names, kept as written on the key cap inside a shortcut ("Ctrl+Shift+Enter"), not in prose. */
const KEY_NAMES = new Set("Ctrl Shift Alt Cmd Esc Meta Fn Enter Tab Space PgUp PgDn F1 F2 F3 F4 F5 F6 F7 F8 F9 F10 F11 F12".split(" "));

/**
 * Terms that used to keep their capitals (acronyms, formats, protocols, units, camel-cased
 * names). Now written lowercase like any word; `uiCaseScan.ts` flags them in any string
 * literal of the UI (not only the ones it recognizes as UI text), so they can't come back.
 */
export const UPPERCASE_TERMS = new Set(
  [
    "MIDI MPE VST VST2 VST3 CLAP AU AUv2 AAX LV2 DAW GPU RAM AI IA MCP URL ID ID3 UUID API HTTP HTTPS",
    "WAV AIFF AIF FLAC MP3 OGG MP4 M4A AAC PCM SFZ SF2 JSON CSV PDF",
    "CC NRPN RPN LFO ADSR AHDSR FX FM RMS LUFS dBFS dBTP IR IRs FFT DC HPF LPF BPF GR PWM VCA MSEG",
    "OSC USB ASIO WASAPI CoreAudio ALSA PipeWire PulseAudio SoundFont DAWproject",
    "SMPTE MTC QR LAN P2P SDK CLI LLM ICE SDP DTLS ULID COOP COEP OPFS CORS UI OS IP DNS TLS HTML PNG SVG ZIP",
    "dB Hz kHz kB KB MB GB TB macOS iOS",
  ]
    .join(" ")
    .split(/\s+/),
);

/** Camel-cased names, flagged in UI text although camel case otherwise reads as a code identifier. */
const CAMEL_NAMES = new Set("OpenAI OpenRouter DeepSeek GitHub AppImage xAI".split(" "));

const WORD = /[A-Za-z_][A-Za-z0-9_]*(?:-[A-Za-z0-9_]+)*/g;

/** Whether `word` (at `index` in `text`) keeps its capitals. */
function allowed(word: string, text: string, index: number): boolean {
  if (!/[A-Z]/.test(word)) return true;
  if (ALLOWED_WORDS.has(word)) return true;
  if (UPPERCASE_TERMS.has(word) || CAMEL_NAMES.has(word)) return false;
  // an identifier (`STATIC_SHARE_ENABLED`, `eth_project`)
  if (word.includes("_")) return true;
  // a code identifier ("WasmTransport", "SharedArrayBuffer")
  if (/[a-z][A-Z]/.test(word)) return true;
  // a key in a shortcut ("⌘K", "⇧A", "Ctrl+Z", "Shift+Enter")
  const before = text.slice(Math.max(0, index - 8), index);
  if (KEY_NAMES.has(word) && text[index + word.length] === "+") return true;
  if (/^[A-Z0-9]$/.test(word) && /[⌘⌥⇧⌃+]\s?$/.test(before)) return true;
  if (/^[A-Z][a-z0-9]+$/.test(word) && /(Ctrl|Shift|Alt|Cmd|Meta|[⌘⌥⇧⌃])\s?\+?\s?$/.test(before)) return true;
  // a note name ("C4", "Bb3", "F#2", "C major")
  if (/^[A-G](b|-?\d+|b-?\d+)?$/.test(word) && (/\d/.test(word) || /^[A-G]b?(#|\d|\s+(major|minor)\b)/.test(text.slice(index)))) return true;
  return false;
}

/** The words of `text` that would be lowercased, with their offsets. A word right after a
 * backslash starts after the escape letter (`\nSave` is "Save", `\u00A0` is "00A0"). */
function badMatches(text: string): Array<{ word: string; index: number }> {
  const out: Array<{ word: string; index: number }> = [];
  for (const m of text.matchAll(WORD)) {
    let [word, index] = [m[0], m.index];
    if (text[index - 1] === "\\") [word, index] = [word.slice(1), index + 1];
    if (word && !allowed(word, text, index)) out.push({ word, index });
  }
  return out;
}

/** The capitalized words of `text` that are not allowed. */
export function badWords(text: string): string[] {
  return badMatches(text).map((m) => m.word);
}

/** `text` with every capitalized word that is not allowed lowercased. */
export function lowerUiText(text: string): string {
  let out = "";
  let last = 0;
  for (const { word, index } of badMatches(text)) {
    out += text.slice(last, index) + word.toLowerCase();
    last = index + word.length;
  }
  return out + text.slice(last);
}

/** The terms of `UPPERCASE_TERMS` in `text`, whatever the syntax around it (a code literal included). */
export function uppercaseTerms(text: string): string[] {
  return [...text.matchAll(/[A-Za-z][A-Za-z0-9]*/g)].map((m) => m[0]).filter((w) => UPPERCASE_TERMS.has(w));
}
