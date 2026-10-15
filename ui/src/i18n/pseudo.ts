/**
 * The pseudo-locale `en-XA` (docs/LOCALIZATION.md): English with accented letters, ~35 %
 * longer and wrapped in ⟦ ⟧. Built at runtime from the English catalog (never a file). Text
 * on screen without the brackets wasn't translated; a clipped bracket is an overflow.
 */
import type { MessageFormatElement } from "@formatjs/icu-messageformat-parser";
import type { CompiledCatalog, CompiledMessage } from "./types";

export const PSEUDO_OPEN = "⟦";
export const PSEUDO_CLOSE = "⟧";
/** How much longer than English (French runs 20–30 % longer). */
const EXPANSION = 0.35;
const PAD = "·";

const ACCENTS: Record<string, string> = {
  a: "á",
  b: "ƀ",
  c: "ç",
  d: "ð",
  e: "é",
  f: "ƒ",
  g: "ĝ",
  h: "ĥ",
  i: "î",
  j: "ĵ",
  k: "ķ",
  l: "ļ",
  m: "ɱ",
  n: "ñ",
  o: "ö",
  p: "þ",
  q: "ǫ",
  r: "ŕ",
  s: "š",
  t: "ţ",
  u: "û",
  v: "ṽ",
  w: "ŵ",
  x: "ẋ",
  y: "ý",
  z: "ž",
  A: "Á",
  B: "Ɓ",
  C: "Ç",
  D: "Ð",
  E: "É",
  F: "Ƒ",
  G: "Ĝ",
  H: "Ĥ",
  I: "Î",
  J: "Ĵ",
  K: "Ķ",
  L: "Ļ",
  M: "Ṁ",
  N: "Ñ",
  O: "Ö",
  P: "Þ",
  Q: "Ǫ",
  R: "Ŕ",
  S: "Š",
  T: "Ţ",
  U: "Û",
  V: "Ṽ",
  W: "Ŵ",
  X: "Ẋ",
  Y: "Ý",
  Z: "Ž",
};

/** Accents every ASCII letter of `text`. */
export function pseudoText(text: string): string {
  let out = "";
  for (const ch of text) out += ACCENTS[ch] ?? ch;
  return out;
}

/** Literal length of a message (what the expansion pads). */
function literalLength(ast: MessageFormatElement[]): number {
  let n = 0;
  for (const el of ast) {
    if (el.type === 0) n += el.value.length;
    else if (el.type === 8) n += literalLength(el.children);
    else if (el.type === 5 || el.type === 6) {
      const first = Object.values(el.options)[0];
      if (first) n += literalLength(first.value);
    }
  }
  return n;
}

function accentAst(ast: MessageFormatElement[]): MessageFormatElement[] {
  return ast.map((el): MessageFormatElement => {
    if (el.type === 0) return { ...el, value: pseudoText(el.value) };
    if (el.type === 8) return { ...el, children: accentAst(el.children) };
    if (el.type === 5 || el.type === 6) {
      const options = Object.fromEntries(Object.entries(el.options).map(([k, o]) => [k, { ...o, value: accentAst(o.value) }]));
      return { ...el, options } as MessageFormatElement;
    }
    return el;
  });
}

function lit(value: string): MessageFormatElement {
  return { type: 0, value } as MessageFormatElement;
}

/** One message in the pseudo-locale. */
export function pseudoMessage(msg: CompiledMessage): CompiledMessage {
  if (typeof msg === "string") {
    if (msg === "") return msg;
    return `${PSEUDO_OPEN}${pseudoText(msg)}${PAD.repeat(Math.ceil(msg.length * EXPANSION))}${PSEUDO_CLOSE}`;
  }
  const pad = PAD.repeat(Math.ceil(literalLength(msg) * EXPANSION));
  return [lit(PSEUDO_OPEN), ...accentAst(msg), lit(`${pad}${PSEUDO_CLOSE}`)];
}

export function pseudoCatalog(en: CompiledCatalog): CompiledCatalog {
  return Object.fromEntries(Object.entries(en).map(([k, m]) => [k, pseudoMessage(m)]));
}
