/**
 * Gettext `.po` round trip for translators whose tool prefers it (`pnpm i18n po fr`,
 * `pnpm i18n po-import fr file.po`). msgctxt is the key, msgid the English message, msgstr
 * the translation (ICU syntax kept as is).
 */
import type { Catalog } from "./check.ts";

const esc = (s: string) => s.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n");
const unesc = (s: string) => s.replace(/\\(["\\n])/g, (_, c: string) => (c === "n" ? "\n" : c));

export function toPo(source: Catalog, target: Catalog, lang: string): string {
  const out = [`msgid ""`, `msgstr ""`, `"Language: ${lang}\\n"`, `"Content-Type: text/plain; charset=UTF-8\\n"`, ""];
  for (const [key, msg] of Object.entries(source)) {
    out.push(`msgctxt "${esc(key)}"`, `msgid "${esc(msg)}"`, `msgstr "${esc(target[key] ?? "")}"`, "");
  }
  return out.join("\n");
}

/** The translated entries of a `.po` file (empty msgstr = untranslated, left out). */
export function fromPo(po: string): Catalog {
  const out: Catalog = {};
  let ctx: string | null = null;
  let field: "ctx" | "id" | "str" | null = null;
  let str = "";
  const flush = () => {
    if (ctx !== null && str !== "") out[ctx] = str;
    ctx = null;
    str = "";
    field = null;
  };
  for (const raw of po.split("\n")) {
    const line = raw.trim();
    const m = /^(msgctxt|msgid|msgstr)\s+"(.*)"$/.exec(line);
    if (m) {
      if (m[1] === "msgctxt") {
        flush();
        ctx = unesc(m[2]!);
        field = "ctx";
      } else if (m[1] === "msgid") field = "id";
      else {
        field = "str";
        str = unesc(m[2]!);
      }
    } else if (/^"(.*)"$/.test(line)) {
      const part = unesc(line.slice(1, -1));
      if (field === "str") str += part;
      else if (field === "ctx" && ctx !== null) ctx += part;
    } else if (line === "") flush();
  }
  flush();
  return out;
}
