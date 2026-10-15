// The user docs' Markdown (docs-site/README.md): a small, strict subset parsed into blocks
// that `DocsMarkdown.tsx` renders as React elements (no HTML strings). Front-matter, headings with
// stable ids, paragraphs, lists, callouts, tables, code, images and Ethereal's inline
// extensions: `[[action:<keymap id>]]`, `[[key:<chord>]]`, `[[desktop]]` / `[[web]]` /
// `[[mobile]]` badges, and the `[[shortcuts]]` block.

export type Platform = "desktop" | "web" | "mobile";
export const PLATFORMS: ReadonlyArray<Platform> = ["desktop", "web", "mobile"];

export type Inline =
  | { t: "text"; v: string }
  | { t: "strong"; c: Inline[] }
  | { t: "em"; c: Inline[] }
  | { t: "code"; v: string }
  | { t: "link"; href: string; c: Inline[] }
  | { t: "action"; id: string }
  | { t: "key"; chord: string }
  | { t: "badge"; platform: Platform };

export type CalloutKind = "tip" | "note" | "warning";

export type Block =
  | { t: "heading"; level: 2 | 3 | 4; id: string; c: Inline[] }
  | { t: "p"; c: Inline[] }
  | { t: "list"; ordered: boolean; items: Block[][] }
  | { t: "callout"; kind: CalloutKind; blocks: Block[] }
  | { t: "table"; head: Inline[][]; rows: Inline[][][] }
  | { t: "code"; v: string }
  | { t: "image"; src: string; alt: string; caption?: string }
  | { t: "shortcuts" };

// ---- Front-matter ------------------------------------------------------------------------

export type FrontMatterValue = string | string[];

/** Splits `---` front-matter (`key: value` and `key:` + `  - item` lists) from the body. */
export function splitFrontMatter(src: string): { data: Record<string, FrontMatterValue>; body: string } {
  const text = src.replace(/\r\n?/g, "\n");
  const data: Record<string, FrontMatterValue> = {};
  if (!text.startsWith("---\n")) return { data, body: text };
  const end = text.indexOf("\n---", 4);
  if (end < 0) return { data, body: text };
  let list: string | null = null;
  for (const line of text.slice(4, end).split("\n")) {
    if (!line.trim() || line.trim().startsWith("#")) continue;
    const item = /^\s+-\s+(.*)$/.exec(line);
    if (item && list) {
      (data[list] as string[]).push(item[1]!.trim());
      continue;
    }
    const kv = /^([A-Za-z_][\w-]*)\s*:\s*(.*)$/.exec(line);
    if (!kv) throw new Error(`front-matter: can't read "${line}"`);
    const [, key, value] = kv as unknown as [string, string, string];
    if (value === "") {
      data[key] = [];
      list = key;
    } else {
      data[key] = unquote(value.trim());
      list = null;
    }
  }
  const after = text.indexOf("\n", end + 4);
  return { data, body: after < 0 ? "" : text.slice(after + 1) };
}

function unquote(v: string): string {
  return /^(["']).*\1$/.test(v) ? v.slice(1, -1) : v;
}

// ---- Ids ---------------------------------------------------------------------------------

/** A heading's id: lowercase, `a-z 0-9 -` only, spaces to `-`. */
export function slugify(text: string): string {
  return text
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9 -]/g, "")
    .trim()
    .replace(/[\s-]+/g, "-")
    .replace(/^-|-$/g, "");
}

// ---- Inline ------------------------------------------------------------------------------

const INLINE = /\\([\\`*_[\]()#!|>-])|`([^`]+)`|\[\[([^\]]+)\]\]|\[((?:[^\]\\]|\\.)+)\]\(([^)\s]+)\)|\*\*((?:[^*\\]|\\.|\*(?!\*))+)\*\*|\*((?:[^*\\]|\\.)+)\*/g;

/** Inline Markdown to nodes. Unknown `[[…]]` extensions throw (the content test names them). */
export function parseInline(src: string): Inline[] {
  const out: Inline[] = [];
  const text = (v: string) => {
    if (!v) return;
    const last = out[out.length - 1];
    if (last?.t === "text") last.v += v;
    else out.push({ t: "text", v });
  };
  let at = 0;
  for (const m of src.matchAll(INLINE)) {
    text(src.slice(at, m.index));
    at = m.index + m[0].length;
    const [, escaped, code, ext, linkText, href, strong, em] = m;
    if (escaped !== undefined) text(escaped);
    else if (code !== undefined) out.push({ t: "code", v: code });
    else if (ext !== undefined) out.push(parseExtension(ext));
    else if (linkText !== undefined) out.push({ t: "link", href: href!, c: parseInline(linkText) });
    else if (strong !== undefined) out.push({ t: "strong", c: parseInline(strong) });
    else if (em !== undefined) out.push({ t: "em", c: parseInline(em) });
  }
  text(src.slice(at));
  return out;
}

function parseExtension(ext: string): Inline {
  const [kind, ...rest] = ext.split(":");
  const arg = rest.join(":").trim();
  if (kind === "action" && arg) return { t: "action", id: arg };
  if (kind === "key" && arg) return { t: "key", chord: arg };
  if ((PLATFORMS as readonly string[]).includes(ext)) return { t: "badge", platform: ext as Platform };
  throw new Error(`unknown extension [[${ext}]]`);
}

/** The plain text of inline nodes (search, ids). Badges are left out. */
export function inlineText(nodes: ReadonlyArray<Inline>, keyText: (n: Inline & { t: "action" | "key" }) => string = defaultKeyText): string {
  return nodes
    .map((n) => {
      switch (n.t) {
        case "text":
        case "code":
          return n.v;
        case "strong":
        case "em":
        case "link":
          return inlineText(n.c, keyText);
        case "action":
        case "key":
          return keyText(n);
        case "badge":
          return "";
      }
    })
    .join("");
}

const defaultKeyText = (n: Inline & { t: "action" | "key" }) => (n.t === "key" ? n.chord : "");

// ---- Blocks ------------------------------------------------------------------------------

const HEADING = /^(#{2,4})\s+(.*?)\s*(?:\{#([a-z0-9-]+)\})?\s*$/;
const LIST_ITEM = /^( *)([-*]|\d+\.)\s+(.*)$/;
const IMAGE = /^!\[([^\]]*)\]\(([^)\s]+)(?:\s+"([^"]*)")?\)\s*$/;
const FENCE = /^```/;
const TABLE_RULE = /^\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/;

/** Parses the body into blocks. Heading ids are unique within the page (`-2`, `-3`…). */
export function parseBlocks(body: string): Block[] {
  const ids = new Map<string, number>();
  const blocks = parseLines(body.replace(/\r\n?/g, "\n").split("\n"));
  const fix = (list: Block[]) => {
    for (const b of list) {
      if (b.t === "heading") {
        const n = ids.get(b.id) ?? 0;
        ids.set(b.id, n + 1);
        if (n > 0) b.id = `${b.id}-${n + 1}`;
      } else if (b.t === "callout") fix(b.blocks);
      else if (b.t === "list") b.items.forEach(fix);
    }
  };
  fix(blocks);
  return blocks;
}

function startsBlock(line: string, next: string | undefined): boolean {
  return (
    HEADING.test(line) ||
    FENCE.test(line) ||
    line.startsWith(">") ||
    IMAGE.test(line) ||
    line.trim() === "[[shortcuts]]" ||
    LIST_ITEM.test(line) ||
    (line.startsWith("|") && next !== undefined && TABLE_RULE.test(next))
  );
}

function parseLines(lines: ReadonlyArray<string>): Block[] {
  const out: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i]!;
    if (!line.trim()) {
      i++;
      continue;
    }
    if (FENCE.test(line)) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !FENCE.test(lines[i]!)) code.push(lines[i++]!);
      i++;
      out.push({ t: "code", v: code.join("\n") });
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      const c = parseInline(h[2]!);
      out.push({ t: "heading", level: h[1]!.length as 2 | 3 | 4, id: h[3] ?? slugify(inlineText(c)), c });
      i++;
      continue;
    }
    if (line.startsWith(">")) {
      const inner: string[] = [];
      while (i < lines.length && lines[i]!.startsWith(">")) inner.push(lines[i++]!.replace(/^> ?/, ""));
      const marker = /^\[!(tip|note|warning)\]\s*$/i.exec(inner[0] ?? "");
      if (marker) inner.shift();
      out.push({ t: "callout", kind: (marker?.[1]?.toLowerCase() ?? "note") as CalloutKind, blocks: parseLines(inner) });
      continue;
    }
    const img = IMAGE.exec(line);
    if (img) {
      out.push({ t: "image", alt: img[1]!, src: img[2]!, ...(img[3] ? { caption: img[3] } : {}) });
      i++;
      continue;
    }
    if (line.trim() === "[[shortcuts]]") {
      out.push({ t: "shortcuts" });
      i++;
      continue;
    }
    if (line.startsWith("|") && i + 1 < lines.length && TABLE_RULE.test(lines[i + 1]!)) {
      const head = cells(line);
      i += 2;
      const rows: Inline[][][] = [];
      while (i < lines.length && lines[i]!.startsWith("|")) rows.push(cells(lines[i++]!));
      out.push({ t: "table", head, rows });
      continue;
    }
    const li = LIST_ITEM.exec(line);
    if (li) {
      i = parseList(lines, i, li[1]!.length, out);
      continue;
    }
    const para: string[] = [];
    while (i < lines.length && lines[i]!.trim() && (para.length === 0 || !startsBlock(lines[i]!, lines[i + 1]))) para.push(lines[i++]!.trim());
    out.push({ t: "p", c: parseInline(para.join(" ")) });
  }
  return out;
}

function cells(row: string): Inline[][] {
  const inner = row.trim().replace(/^\|/, "").replace(/\|$/, "");
  // Split on pipes that aren't escaped (`\|`).
  return inner.split(/(?<!\\)\|/).map((c) => parseInline(c.trim()));
}

/** A list starting at `start` (items at `indent`); pushes it to `out`, returns the next line. */
function parseList(lines: ReadonlyArray<string>, start: number, indent: number, out: Block[]): number {
  const first = LIST_ITEM.exec(lines[start]!)!;
  const ordered = /\d/.test(first[2]!);
  const items: Block[][] = [];
  let i = start;
  while (i < lines.length) {
    const m = LIST_ITEM.exec(lines[i]!);
    if (!m || m[1]!.length !== indent || /\d/.test(m[2]!) !== ordered) break;
    const body = [m[3]!];
    const inner = indent + m[2]!.length + 1;
    i++;
    while (i < lines.length) {
      const l = lines[i]!;
      if (!l.trim()) {
        // A blank line continues the item only if the next line is indented into it.
        const next = lines[i + 1];
        if (next !== undefined && next.trim() && leading(next) >= Math.min(inner, indent + 2)) {
          body.push("");
          i++;
          continue;
        }
        break;
      }
      if (leading(l) >= Math.min(inner, indent + 2)) body.push(l.slice(Math.min(leading(l), inner)));
      else if (!LIST_ITEM.test(l) && !startsBlock(l, lines[i + 1]) && body[body.length - 1] !== "") body.push(l.trim());
      else break;
      i++;
    }
    items.push(parseLines(body));
  }
  out.push({ t: "list", ordered, items });
  return i;
}

const leading = (l: string) => l.length - l.trimStart().length;

/** The plain text of blocks (the search index). */
export function blocksText(blocks: ReadonlyArray<Block>, keyText?: (n: Inline & { t: "action" | "key" }) => string): string {
  const parts: string[] = [];
  const walk = (list: ReadonlyArray<Block>) => {
    for (const b of list) {
      switch (b.t) {
        case "heading":
        case "p":
          parts.push(inlineText(b.c, keyText));
          break;
        case "list":
          b.items.forEach(walk);
          break;
        case "callout":
          walk(b.blocks);
          break;
        case "table":
          for (const row of [b.head, ...b.rows]) parts.push(row.map((c) => inlineText(c, keyText)).join(" "));
          break;
        case "code":
          parts.push(b.v);
          break;
        case "image":
          parts.push(b.caption ?? b.alt);
          break;
        case "shortcuts":
          break;
      }
    }
  };
  walk(blocks);
  return parts.join("\n");
}
