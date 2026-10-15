// The rendered side of the lowercase rule (#524): text the UI shows on screen. A text node is
// the UI's own unless it sits in user content (`.eth-user-text`: names of tracks, clips,
// devices, presets, files, people...) or in a field. `vitest.setup.ts` checks every test's
// final DOM with it, which catches what the source scan can't see (an enum value rendered
// as is, a label built at run time).
import { badWords } from "./uiCase";

const SKIP = "script, style, input, textarea, kbd, code, [contenteditable='true'], .eth-user-text";

/** The capitalized words of the UI's own text under `root`, with their text. */
export function domCaseFindings(root: Element): Array<{ word: string; text: string }> {
  const out: Array<{ word: string; text: string }> = [];
  const walker = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const text = n.nodeValue ?? "";
    if (!/[A-Z]/.test(text)) continue;
    const parent = n.parentElement;
    if (!parent || parent.closest(SKIP)) continue;
    for (const word of badWords(text)) out.push({ word, text: text.trim().slice(0, 80) });
  }
  return out;
}
