/**
 * The library file rules (regular expressions on file names, case ignored) in plain terms, for
 * people who don't write regex: a file type (extensions), or a name that starts with, ends
 * with, is or contains some text. `fromRegex` reads a rule as one of those (null when it is
 * anything else: a custom pattern); `toRegex` writes one back.
 */

export type RuleKind = "type" | "starts" | "ends" | "is" | "contains";

export interface FriendlyRule {
  kind: RuleKind;
  /** The text; for a file type, extensions separated by commas ("wav, aif"). */
  value: string;
}

export const RULE_KINDS: ReadonlyArray<RuleKind> = ["type", "starts", "ends", "is", "contains"];

const SPECIAL = new Set([..."\\^$.|?*+()[]{}"]);

const escape = (text: string): string => [...text].map((c) => (SPECIAL.has(c) ? `\\${c}` : c)).join("");

/** The literal text a regex fragment matches, or null when it isn't plain text (escaped specials allowed). */
function literal(src: string): string | null {
  let out = "";
  for (let i = 0; i < src.length; i++) {
    const c = src[i]!;
    if (c === "\\") {
      const next = src[i + 1];
      if (next === undefined || !SPECIAL.has(next)) return null;
      out += next;
      i++;
    } else if (SPECIAL.has(c)) {
      return null;
    } else {
      out += c;
    }
  }
  return out;
}

const EXTENSION = /^[a-z0-9_-]+$/i;

/** `rule` as a friendly rule, or null (a custom pattern). */
export function fromRegex(rule: string): FriendlyRule | null {
  // A file type: `\.ext$` or `\.(a|b|c)$`.
  const many = /^\\\.\(([^()]*)\)\$$/.exec(rule);
  if (many) {
    const exts = many[1]!.split("|");
    if (exts.length > 0 && exts.every((e) => EXTENSION.test(e))) return { kind: "type", value: exts.join(", ") };
  }
  const one = /^\\\.([a-z0-9_-]+)\$$/i.exec(rule);
  if (one) return { kind: "type", value: one[1]! };
  const anchoredStart = rule.startsWith("^");
  const anchoredEnd = rule.endsWith("$") && !rule.endsWith("\\$");
  const body = rule.slice(anchoredStart ? 1 : 0, anchoredEnd ? -1 : undefined);
  const text = literal(body);
  if (text === null || text === "") return null;
  if (anchoredStart && anchoredEnd) return { kind: "is", value: text };
  if (anchoredStart) return { kind: "starts", value: text };
  if (anchoredEnd) return { kind: "ends", value: text };
  return { kind: "contains", value: text };
}

/** The extensions of a file type's text ("wav, .aif" → ["wav", "aif"]). */
export const extensionsOf = (value: string): string[] =>
  value
    .split(/[\s,;]+/)
    .map((e) => e.replace(/^\.+/, "").trim())
    .filter(Boolean);

/** `r` as a regex ("" when it has nothing to match yet). */
export function toRegex(r: FriendlyRule): string {
  if (r.kind === "type") {
    const exts = extensionsOf(r.value).map(escape);
    if (exts.length === 0) return "";
    return exts.length === 1 ? `\\.${exts[0]}$` : `\\.(${exts.join("|")})$`;
  }
  if (r.value === "") return "";
  const text = escape(r.value);
  switch (r.kind) {
    case "starts":
      return `^${text}`;
    case "ends":
      return `${text}$`;
    case "is":
      return `^${text}$`;
    case "contains":
      return text;
  }
}
