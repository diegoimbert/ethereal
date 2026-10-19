// @vitest-environment node
// The docs translations (docs-site/content-<lang>): a translated page is the English page in
// another language, so it keeps the English page's structure. Same front-matter (order,
// platforms, audience, coverage), the same heading ids in the same order (translated headings
// keep the English id with `{#id}`, so links from other pages still land), the same shortcuts,
// lowercase titles and headings, and the app's labels as that language's catalog has them.
import { describe, expect, it } from "vitest";
import { readCatalog } from "@/i18n/tooling/check";
import { contentFiles, getDocs, loadDocs, staleTranslations } from "./content";
import { files as fr } from "./contentFr";
import { inlineText, type Block, type Inline } from "./markdown";
import { buildDocs, type DocPage } from "./model";
import { docsLinkTarget } from "./route";

const english = getDocs("en");
const TRANSLATIONS = { fr };

function inlines(page: DocPage): Inline[] {
  const out: Inline[] = [];
  const walkInline = (list: ReadonlyArray<Inline>) => {
    for (const n of list) {
      out.push(n);
      if (n.t === "strong" || n.t === "em" || n.t === "link") walkInline(n.c);
    }
  };
  const walk = (list: ReadonlyArray<Block>) => {
    for (const b of list) {
      if (b.t === "heading" || b.t === "p") walkInline(b.c);
      else if (b.t === "list") b.items.forEach(walk);
      else if (b.t === "callout") walk(b.blocks);
      else if (b.t === "table") [b.head, ...b.rows].forEach((row) => row.forEach(walkInline));
    }
  };
  walk(page.blocks);
  return out;
}

/** The shortcuts a page shows, sorted (translations may reorder a sentence). */
const shortcuts = (page: DocPage) =>
  inlines(page)
    .flatMap((n) => (n.t === "action" ? [`action:${n.id}`] : n.t === "key" ? [`key:${n.chord}`] : []))
    .sort();

/** Case, spaces and no-break spaces aside. */
const normLabel = (s: string) =>
  s
    .replace(/[\u00a0\u202f\s]+/g, " ")
    .trim()
    .toLowerCase();

/** The bold spans of a page: where the docs quote the app's labels. */
const boldTexts = (page: DocPage) => inlines(page).flatMap((n) => (n.t === "strong" ? [normLabel(inlineText(n.c))] : []));

/**
 * Bold words of an English page that are also a catalog label but aren't the app's label in
 * that sentence (a verb, a list heading, a gesture, another DAW),
 * so their translation needn't be that label's. Built-in device text counts as labels
 * (`engine.*`).
 */
const NOT_LABELS: Record<string, string[]> = {
  "basics/projects.md": ["type"],
  "basics/tempo-and-time-signature.md": ["type"],
  // "share" there is the verb ("partager" in French), not the catalog's noun label.
  "basics/the-screen.md": ["share"],
  "import-export/other-daws.md": ["waveform"],
  "import-export/exporting-to-other-daws.md": ["take lanes"],
  "making-music/automation.md": ["off"],
  "making-music/expression-and-mpe.md": ["draw"],
  "making-music/mixing.md": ["send"],
  // "group track" there is the menu entry (a plural message), not the track kind.
  "making-music/tracks.md": ["group track"],
  "mobile/editing-on-touch.md": ["tap"],
  "mobile/gestures.md": ["tap"],
  "mobile/mobile-beta.md": ["devices", "recording", "share"],
  "reference/faq.md": ["share"],
  "reference/glossary.md": ["plugin"],
  "sharing/live-sessions.md": ["share"],
  "sharing/overview.md": ["share"],
  "sharing/static-links.md": ["share"],
  "welcome/what-is-ethereal.md": ["plugins", "recording"],
};

describe.each(Object.entries(TRANSLATIONS))("docs translation %s", (lang, files) => {
  const docs = buildDocs({ ...contentFiles(), ...files });
  const translated = docs.pages.filter((p) => files[p.file] !== undefined);

  it("translates pages and sections that exist in English", () => {
    const english = contentFiles();
    expect(Object.keys(files).filter((f) => english[f] === undefined)).toEqual([]);
    expect(translated.length).toBeGreaterThan(0);
  });

  it("is up to date with the English pages", () => {
    // An English page gained, lost or renamed a heading (or its coverage changed): the app shows
    // it in English until its translation catches up. Translate the change (docs/LOCALIZATION.md "Docs").
    expect(staleTranslations(files), `docs-site/content-${lang}: translations behind their English page`).toEqual([]);
  });

  it("keeps each page's front-matter, heading ids and shortcuts", () => {
    for (const page of translated) {
      const en = english.byPath.get(page.path)!;
      const shape = (p: DocPage) => ({ order: p.order, platforms: p.platforms, audience: p.audience, covers: p.covers, headings: p.headings.map((h) => h.id) });
      expect(shape(page), page.file).toEqual(shape(en));
      expect(shortcuts(page), page.file).toEqual(shortcuts(en));
      expect(page.summary, page.file).not.toBe("");
    }
    for (const s of docs.sections) expect(s.order, s.slug).toBe(english.sections.find((e) => e.slug === s.slug)!.order);
  });

  it("links resolve", () => {
    const broken: string[] = [];
    for (const page of translated)
      for (const n of inlines(page)) {
        if (n.t !== "link") continue;
        const target = n.href.startsWith("#") ? { path: page.path, anchor: n.href.slice(1) } : docsLinkTarget(n.href);
        if (!target || !target.path) continue;
        const to = docs.byPath.get(target.path) ?? docs.sections.find((s) => s.slug === target.path)?.pages[0];
        if (!to) broken.push(`${page.file}: ${n.href}`);
        else if (target.anchor && !to.headings.some((h) => h.id === target.anchor)) broken.push(`${page.file}: ${n.href} (no such heading)`);
      }
    expect(broken).toEqual([]);
  });

  it("titles and headings are lowercase", () => {
    const bad: string[] = [];
    for (const page of translated) {
      if (page.title !== page.title.toLowerCase()) bad.push(`${page.file}: title "${page.title}"`);
      for (const h of page.headings) {
        const text = inlineText(h.c.filter((n) => n.t !== "code"));
        if (text !== text.toLowerCase()) bad.push(`${page.file}: heading "${text}"`);
      }
    }
    for (const s of docs.sections) if (s.title !== s.title.toLowerCase()) bad.push(`${s.slug}: section "${s.title}"`);
    expect(bad).toEqual([]);
  });

  it.runIf(lang === "fr")("follows French typography: ’ and no-break spaces", () => {
    const bad: string[] = [];
    for (const page of translated) {
      const texts = [page.title, page.summary, ...inlines(page).flatMap((n) => (n.t === "text" ? [n.v] : []))];
      for (const v of texts) if (/'| [:;?!»]|« /.test(v)) bad.push(`${page.file}: "${v}"`);
    }
    expect(bad).toEqual([]);
  });

  it("quotes the app's labels as that language's catalog has them", () => {
    // A label the English page quotes in bold (an English catalog message) is quoted in bold in
    // the translation as that message's translation: what a reader sees in the app.
    const en = readCatalog("en");
    const tr = readCatalog(lang);
    const byEnglish = new Map<string, Set<string>>();
    for (const [key, text] of Object.entries(en)) {
      if (text.includes("{") || tr[key] === undefined) continue;
      const k = normLabel(text);
      byEnglish.set(k, (byEnglish.get(k) ?? new Set()).add(normLabel(tr[key])));
    }
    const bad: string[] = [];
    for (const page of translated) {
      const quoted = new Set(boldTexts(page));
      const skip = new Set(NOT_LABELS[page.file] ?? []);
      for (const label of new Set(boldTexts(english.byPath.get(page.path)!))) {
        const translations = byEnglish.get(label);
        if (!translations || skip.has(label) || [...translations].some((t) => quoted.has(t))) continue;
        bad.push(`${page.file}: **${label}** → **${[...translations].join("** or **")}**`);
      }
    }
    expect(bad).toEqual([]);
  });

  it("is what the app shows in that language", async () => {
    await loadDocs(lang as "fr");
    const shown = getDocs(lang as "fr");
    for (const page of translated) expect(shown.byPath.get(page.path)!.title).toBe(page.title);
  });
});

describe("a stale translation", () => {
  it("shows in English, so links and anchors keep working", () => {
    const page = "welcome/what-is-ethereal.md";
    const behind = fr[page]!.replace(/\n## le bandeau ouvrir dans l’application[^]*?(?=\n## )/, "\n");
    expect(behind).not.toBe(fr[page]);
    expect(staleTranslations({ [page]: behind, "welcome/first-beat.md": fr["welcome/first-beat.md"]! })).toEqual([page]);
    expect(staleTranslations({ "welcome/gone.md": "---\ntitle: x\norder: 1\nsummary: x\n---\n" })).toEqual(["welcome/gone.md"]);
  });

  it("is caught when it drops a link or a paragraph the English page has", () => {
    const page = "basics/the-screen.md";
    const src = fr[page]!;
    const link = /\[([^\]]+)\]\(\/docs\/making-music\/clips#double-clicking-a-clip\)/;
    expect(src).toMatch(link);
    expect(staleTranslations({ [page]: src.replace(link, "$1") })).toEqual([page]);
    const para = src.indexOf("\n\n", src.indexOf("\n## ")) + 2;
    const dropped = src.slice(0, para) + src.slice(src.indexOf("\n\n", para) + 2);
    expect(staleTranslations({ [page]: dropped })).toEqual([page]);
    expect(staleTranslations({ [page]: src })).toEqual([]);
  });
});
