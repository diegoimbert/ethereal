// The bundled docs: every Markdown file and image under docs-site/content, read at build time
// (Vite globs), parsed once when the docs chunk loads. Translations live beside it
// (docs-site/content-<lang>, a lazy chunk per language): a translated page replaces the
// English one, the others stay English, page by page (docs/LOCALIZATION.md "Docs").
import { useEffect, useState } from "react";
import { getLocale, useLocale, type Locale } from "@/i18n";
import type { Block, Inline } from "./markdown";
import { buildDocs, type Docs, type DocPage } from "./model";

const ROOT = "../../../../docs-site/content/";

const markdown = import.meta.glob("../../../../docs-site/content/**/*.md", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const images = import.meta.glob("../../../../docs-site/content/**/*.{png,webp,jpg,svg}", { query: "?url", import: "default", eager: true }) as Record<
  string,
  string
>;

/** `{ "<section>/<page>.md": source }`. */
export function contentFiles(): Record<string, string> {
  return Object.fromEntries(Object.entries(markdown).map(([k, v]) => [k.slice(ROOT.length), v]));
}

/** The translations, per language (absent: the docs are English only in that language). */
const TRANSLATIONS: Partial<Record<Locale, () => Promise<Record<string, string>>>> = {
  fr: () => import("./contentFr").then((m) => m.files),
};

const loaded = new Map<Locale, Record<string, string>>();
const built = new Map<Locale | "en", Docs>();

/** The docs in `locale` once its translations are loaded ([`loadDocs`]), else in English. */
export function getDocs(locale: Locale = getLocale()): Docs {
  const overlay = loaded.get(locale);
  const key = overlay ? locale : "en";
  let docs = built.get(key);
  if (!docs) {
    docs = buildDocs(overlay ? { ...contentFiles(), ...overlay } : contentFiles());
    built.set(key, docs);
  }
  return docs;
}

/**
 * The shape a translation keeps from its English page: headings (their ids are link targets),
 * coverage, the block structure (a paragraph, list item, table row or callout added or dropped)
 * and the links in each block.
 */
function shape(page: DocPage): string {
  return JSON.stringify([page.headings.map((h) => h.id), page.covers, page.blocks.map(blockShape)]);
}

function blockShape(b: Block): unknown {
  switch (b.t) {
    case "heading":
      return `h:${b.id}`;
    case "p":
      return ["p", linksOf(b.c)];
    case "list":
      return [b.ordered ? "ol" : "ul", b.items.map((item) => item.map(blockShape))];
    case "callout":
      return [b.kind, b.blocks.map(blockShape)];
    case "table":
      return ["table", b.rows.length, [b.head, ...b.rows].map((row) => row.map(linksOf))];
    case "image":
      return `img:${b.src}`;
    default:
      return b.t;
  }
}

/** The link targets in `nodes`, in order. */
function linksOf(nodes: ReadonlyArray<Inline>): string[] {
  const out: string[] = [];
  for (const n of nodes) {
    if (n.t === "link") out.push(n.href, ...linksOf(n.c));
    else if (n.t === "strong" || n.t === "em") out.push(...linksOf(n.c));
  }
  return out;
}

/**
 * The translated files whose English page has changed shape since (a heading added, removed or
 * renamed, or its coverage changed), or that translate no English page. The app shows those pages
 * in English until the translation catches up, so links and anchors never break;
 * translations.test.ts fails on them.
 */
export function staleTranslations(files: Readonly<Record<string, string>>): string[] {
  const english = getDocs("en");
  const en = contentFiles();
  const merged = buildDocs({ ...en, ...files });
  const stale = Object.keys(files).filter((f) => en[f] === undefined);
  for (const page of merged.pages) {
    if (files[page.file] === undefined || en[page.file] === undefined) continue;
    if (shape(english.byPath.get(page.path)!) !== shape(page)) stale.push(page.file);
  }
  return stale;
}

/** Loads `locale`'s translated pages (no-op for a language without docs translations). */
export async function loadDocs(locale: Locale): Promise<void> {
  const load = TRANSLATIONS[locale];
  if (!load || loaded.has(locale)) return;
  const files = { ...(await load()) };
  for (const f of staleTranslations(files)) delete files[f];
  loaded.set(locale, files);
}

/** Does `page` show in `locale` (a translation, or the English docs in English), or in English? */
export function pageIsTranslated(page: DocPage, locale: Locale = getLocale()): boolean {
  if (!TRANSLATIONS[locale]) return true;
  return loaded.get(locale)?.[page.file] !== undefined;
}

/** The docs in the UI's language, following a language switch (English until its pages load). */
export function useDocs(): Docs {
  const locale = useLocale();
  const [, setReady] = useState(0);
  useEffect(() => {
    let live = true;
    void loadDocs(locale).then(() => live && setReady((n) => n + 1));
    return () => {
      live = false;
    };
  }, [locale]);
  return getDocs(locale);
}

/** The URL of an image a page links to (relative to the page's file), or null. */
export function imageUrl(page: DocPage, src: string): string | null {
  const dir = page.file.split("/").slice(0, -1);
  for (const part of src.split("/")) {
    if (part === "..") dir.pop();
    else if (part !== ".") dir.push(part);
  }
  return images[ROOT + dir.join("/")] ?? null;
}

/** Every bundled image (relative to the content root), for the content test. */
export function imageFiles(): string[] {
  return Object.keys(images).map((k) => k.slice(ROOT.length));
}
