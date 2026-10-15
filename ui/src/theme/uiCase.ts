// The UI's own text is lowercase (#524): menus, buttons, dialogs, tooltips, toasts, labels,
// placeholders, empty states, aria-labels. Exceptions keep their capitals: acronyms and units
// (MIDI, VST3, dB, Hz), key names (Ctrl, Shift), and proper nouns (vendors, products,
// platforms). User content (names the user typed, plugin names) is never rewritten.
// `uiCaseScan.ts` checks the source literals against these rules (`uiCase.test.ts`).

/** Words that keep their capitals: acronyms, units, key names, proper nouns, vendors. */
export const ALLOWED_WORDS = new Set(
  [
    // acronyms and formats
    "MIDI MPE VST VST2 VST3 CLAP AU AUv2 AAX LV2 DAW GPU RAM AI MCP URL ID ID3 UUID API HTTP HTTPS",
    "WAV AIFF AIF FLAC MP3 OGG MP4 M4A AAC PCM SFZ SF2 ZIP JSON CSV PDF PNG SVG HTML",
    "CC NRPN RPN LFO ADSR AHDSR EQ FX FM RMS LUFS dBFS dBTP IR FFT DC HPF LPF BPF",
    "OSC USB ASIO WASAPI CoreAudio ALSA JACK PipeWire PulseAudio WebAudio WebGL WebGPU WebRTC WebSocket WASM TURN STUN ICE",
    "SMPTE MTC UI OS QR TLS DNS IP LAN P2P SDK CLI LLM VCA MSEG ASCII UTF-8 SHA-256 SoundFont COOP COEP OPFS CORS SDP DTLS ULID MSI DSP",
    // units
    "dB Hz kHz kbps B kB KB MB GB TB",
    // pan and grid notation ("C", "25L", "100R", "1/8T")
    "C L R T",
    // key names (shortcut glyphs)
    "Ctrl Shift Alt Cmd Esc Meta Fn F1 F2 F3 F4 F5 F6 F7 F8 F9 F10 F11 F12 PgUp PgDn",
    // proper nouns: vendors, products, platforms
    "Ableton Bitwig FL REAPER Reaper Cubase Nuendo Steinberg Apple Microsoft Google Windows macOS Linux iOS Android",
    "Chrome Chromium Firefox Safari GitHub Tauri AppImage Flatpak Anthropic Claude OpenAI OpenRouter DeepSeek Groq Cloudflare Ollama Gemini Mistral xAI Grok",
    "DAWproject Kontakt Serum Xfer Arturia Spitfire Diva Splice Freesound Roland Korg Yamaha Moog Akai MPC",
  ]
    .join(" ")
    .split(/\s+/),
);

/** Multi-word proper nouns whose words would otherwise be lowercase ("Ableton Live", "FL Studio"). */
export const ALLOWED_PHRASES = ["Ableton Live", "FL Studio", "Bitwig Studio", "Studio One", "LM Studio", "Logic Pro", "Pro Tools", "Audio Unit", "Audio Units"];

const WORD = /[A-Za-z_][A-Za-z0-9_]*(?:-[A-Za-z0-9_]+)*/g;

/** Whether `word` (at `index` in `text`) keeps its capitals. */
function allowed(word: string, text: string, index: number): boolean {
  if (!/[A-Z]/.test(word)) return true;
  if (ALLOWED_WORDS.has(word)) return true;
  // an identifier (`STATIC_SHARE_ENABLED`, `eth_project`)
  if (word.includes("_")) return true;
  // a code identifier or a camel-cased brand ("WasmTransport", "OpenRouter", "macOS")
  if (/[a-z][A-Z]/.test(word)) return true;
  // plural acronym ("CCs", "URLs", "LFOs")
  if (/^[A-Z][A-Z0-9]+s$/.test(word) && ALLOWED_WORDS.has(word.slice(0, -1))) return true;
  // compound ("VST3-only", "UTF-8"): every capitalized part is allowed
  if (word.includes("-") && word.split("-").every((p) => !/[A-Z]/.test(p) || ALLOWED_WORDS.has(p))) return true;
  // a key in a shortcut ("⌘K", "⇧A", "Ctrl+Z", "Shift+Enter")
  const before = text.slice(Math.max(0, index - 8), index);
  if (/^[A-Z0-9]$/.test(word) && /[⌘⌥⇧⌃+]\s?$/.test(before)) return true;
  if (/^[A-Z][a-z]+$/.test(word) && /(Ctrl|Shift|Alt|Cmd|Meta|[⌘⌥⇧⌃])\s?\+?\s?$/.test(before)) return true;
  // a note name ("C4", "Bb3", "F#2", "C major")
  if (/^[A-G](b|-?\d+|b-?\d+)?$/.test(word) && (/\d/.test(word) || /^[A-G]b?(#|\d|\s+(major|minor)\b)/.test(text.slice(index)))) return true;
  return false;
}

/** Masks the allowed phrases (so their words are skipped), keeping offsets. */
function maskPhrases(text: string): string {
  let out = text;
  for (const p of ALLOWED_PHRASES) out = out.split(p).join(" ".repeat(p.length));
  return out;
}

/** The words of `text` that would be lowercased, with their offsets. A word right after a
 * backslash starts after the escape letter (`\nSave` is "Save", `\u00A0` is "00A0"). */
function badMatches(text: string): Array<{ word: string; index: number }> {
  const masked = maskPhrases(text);
  const out: Array<{ word: string; index: number }> = [];
  for (const m of masked.matchAll(WORD)) {
    let [word, index] = [m[0], m.index];
    if (masked[index - 1] === "\\") [word, index] = [word.slice(1), index + 1];
    if (word && !allowed(word, masked, index)) out.push({ word, index });
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
