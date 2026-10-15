// Client-side search over the docs: titles, headings and text, no service. The index has one
// entry per page part (the lead, then each heading's part) and is built once from the
// bundled content.
import { blocksText, inlineText, type Block, type Inline } from "./markdown";
import type { Docs, DocPage } from "./model";

export interface SearchEntry {
  path: string;
  /** Heading id, or "" for the page's lead. */
  anchor: string;
  pageTitle: string;
  sectionTitle: string;
  /** The heading, or "" for the lead. */
  heading: string;
  text: string;
  /** Lowercased fields (matching). */
  lc: { title: string; heading: string; text: string };
}

export interface SearchResult {
  entry: SearchEntry;
  score: number;
  /** Text around the first match. */
  snippet: string;
}

/** Text of a key extension in the index: the action's chord words don't help search. */
const keyText = (n: Inline & { t: "action" | "key" }) => (n.t === "key" ? n.chord.replace(/\+/g, " ") : "");

export function buildIndex(docs: Docs): SearchEntry[] {
  const out: SearchEntry[] = [];
  for (const section of docs.sections) for (const page of section.pages) out.push(...pageEntries(page, section.title));
  return out;
}

function pageEntries(page: DocPage, sectionTitle: string): SearchEntry[] {
  const parts: Array<{ anchor: string; heading: string; blocks: Block[] }> = [{ anchor: "", heading: "", blocks: [] }];
  for (const b of page.blocks) {
    if (b.t === "heading") parts.push({ anchor: b.id, heading: inlineText(b.c), blocks: [] });
    else parts[parts.length - 1]!.blocks.push(b);
  }
  parts[0]!.blocks.unshift({ t: "p", c: [{ t: "text", v: page.summary }] });
  return parts.map((p) => {
    const text = blocksText(p.blocks, keyText).replace(/\s+/g, " ").trim();
    return {
      path: page.path,
      anchor: p.anchor,
      pageTitle: page.title,
      sectionTitle,
      heading: p.heading,
      text,
      lc: { title: page.title.toLowerCase(), heading: p.heading.toLowerCase(), text: text.toLowerCase() },
    };
  });
}

/** Query words (lowercase, 1+ letters). */
export function queryTerms(query: string): string[] {
  return query
    .toLowerCase()
    .split(/[^\p{L}\p{N}.+#/-]+/u)
    .map((t) => t.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, ""))
    .filter(Boolean);
}

/** Where `term` starts a word in `text` (-1 if nowhere). */
function wordStart(text: string, term: string): number {
  let i = text.indexOf(term);
  while (i >= 0) {
    if (i === 0 || !/[\p{L}\p{N}]/u.test(text[i - 1]!)) return i;
    i = text.indexOf(term, i + 1);
  }
  return -1;
}

/**
 * The best entries for `query`: every word must start a word somewhere in the entry (its page
 * title, heading or text). Titles weigh most, then headings, then text. At most one result per
 * heading; `limit` results.
 */
export function search(index: ReadonlyArray<SearchEntry>, query: string, limit = 12): SearchResult[] {
  const terms = queryTerms(query);
  if (terms.length === 0) return [];
  const results: SearchResult[] = [];
  for (const entry of index) {
    let score = 0;
    let first = -1;
    let ok = true;
    for (const term of terms) {
      const inTitle = wordStart(entry.lc.title, term) >= 0;
      const inHeading = wordStart(entry.lc.heading, term) >= 0;
      const at = wordStart(entry.lc.text, term);
      if (!inTitle && !inHeading && at < 0) {
        ok = false;
        break;
      }
      score += (inTitle ? 6 : 0) + (inHeading ? 10 : 0) + (at >= 0 ? 1 : 0);
      if (at >= 0 && (first < 0 || at < first)) first = at;
    }
    if (!ok) continue;
    // The whole query as a phrase counts extra.
    const phrase = terms.join(" ");
    if (terms.length > 1 && (entry.lc.heading.includes(phrase) || entry.lc.title.includes(phrase))) score += 8;
    else if (terms.length > 1 && entry.lc.text.includes(phrase)) score += 3;
    // A page's lead matches its title: prefer it to its parts that only repeat the title words.
    if (!entry.anchor) score += 1;
    results.push({ entry, score, snippet: snippet(entry.text, first) });
  }
  results.sort((a, b) => b.score - a.score || a.entry.pageTitle.localeCompare(b.entry.pageTitle));
  return results.slice(0, limit);
}

const SNIPPET = 140;

function snippet(text: string, at: number): string {
  if (text.length <= SNIPPET) return text;
  const start = at < 0 ? 0 : Math.max(0, text.lastIndexOf(" ", Math.max(0, at - 40)) + 1);
  const cut = text.slice(start, start + SNIPPET);
  const end = start + SNIPPET < text.length ? cut.slice(0, cut.lastIndexOf(" ")) + "…" : cut;
  return (start > 0 ? "…" : "") + end;
}
