// @vitest-environment node
// The real docs content (docs-site/content): it parses, links resolve, shortcuts name real
// keymap actions, images are small, and the chrome (titles, headings) is lowercase.
import { readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { isValidChord, MODIFIERS } from "@/features/keymap/chords";
import { getDocs, imageFiles, imageUrl } from "./content";
import { parsePending } from "./coverage";
import { inlineText, type Block, type Inline } from "./markdown";
import type { DocPage } from "./model";
import { DOC_LINKS, docsLinkTarget } from "./route";
import { docAction } from "./shortcuts";

const docs = getDocs();
/** Pages linked to but not written yet (docs-site/pending.txt). */
const pendingPages = new Set(parsePending(readFileSync(fileURLToPath(new URL("../../../../docs-site/pending.txt", import.meta.url)), "utf8")).pages);

/** Every inline node of a page (headings, paragraphs, lists, callouts, tables). */
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

function blocksOf(page: DocPage): Block[] {
  const out: Block[] = [];
  const walk = (list: ReadonlyArray<Block>) => {
    for (const b of list) {
      out.push(b);
      if (b.t === "list") b.items.forEach(walk);
      else if (b.t === "callout") walk(b.blocks);
    }
  };
  walk(page.blocks);
  return out;
}

describe("docs content", () => {
  it("has the sections in reading order, each with pages", () => {
    expect(docs.sections.length).toBeGreaterThan(0);
    expect(docs.sections[0]!.slug).toBe("welcome");
    for (const s of docs.sections) expect(s.pages.length, s.slug).toBeGreaterThan(0);
  });

  it("internal links point to existing pages and headings", () => {
    const broken: string[] = [];
    for (const page of docs.pages)
      for (const n of inlines(page)) {
        if (n.t !== "link") continue;
        if (n.href.startsWith("#")) {
          if (!page.headings.some((h) => h.id === n.href.slice(1))) broken.push(`${page.path}: ${n.href}`);
          continue;
        }
        const target = docsLinkTarget(n.href);
        if (!target) {
          if (!/^(https:\/\/|\/(download|s\/|$))/.test(n.href)) broken.push(`${page.path}: ${n.href} (use /docs/… or https://)`);
          continue;
        }
        const to = target.path ? (docs.byPath.get(target.path) ?? docs.sections.find((s) => s.slug === target.path)?.pages[0]) : docs.pages[0];
        if (!to) {
          if (!pendingPages.has(target.path)) broken.push(`${page.path}: ${n.href} (no such page)`);
        } else if (target.anchor && !to.headings.some((h) => h.id === target.anchor)) broken.push(`${page.path}: ${n.href} (no such heading)`);
      }
    expect(broken).toEqual([]);
  });

  it("shortcuts name real keymap actions and valid keys", () => {
    const bad: string[] = [];
    for (const page of docs.pages)
      for (const n of inlines(page)) {
        if (n.t === "action" && !docAction(n.id)) bad.push(`${page.path}: [[action:${n.id}]]`);
        if (n.t === "key") {
          const parts = n.chord.split("+");
          const modsOnly = parts.every((p) => (MODIFIERS as readonly string[]).includes(p));
          if (!isValidChord(n.chord) && !modsOnly) bad.push(`${page.path}: [[key:${n.chord}]]`);
        }
      }
    expect(bad).toEqual([]);
  });

  it("the generated shortcuts table is on the shortcuts page only", () => {
    const pages = docs.pages.filter((p) => blocksOf(p).some((b) => b.t === "shortcuts")).map((p) => p.path);
    expect(pages).toEqual([DOC_LINKS.shortcuts]);
  });

  it("titles and headings are lowercase, like the app", () => {
    const bad: string[] = [];
    for (const page of docs.pages) {
      if (page.title !== page.title.toLowerCase()) bad.push(`${page.path}: title "${page.title}"`);
      for (const h of page.headings) {
        const text = inlineText(h.c.filter((n) => n.t !== "code"));
        if (text !== text.toLowerCase()) bad.push(`${page.path}: heading "${text}"`);
      }
    }
    for (const s of docs.sections) if (s.title !== s.title.toLowerCase()) bad.push(`${s.slug}: section "${s.title}"`);
    expect(bad).toEqual([]);
  });

  it("every page has a summary and ends with links onward", () => {
    const bad = docs.pages.filter((p) => !p.summary || !p.headings.some((h) => h.id === "where-next")).map((p) => p.path);
    expect(bad, "pages need a summary and a `## where next` section").toEqual([]);
  });

  it("images exist and are small (< 200 KB)", () => {
    const bad: string[] = [];
    for (const page of docs.pages) for (const b of blocksOf(page)) if (b.t === "image" && !imageUrl(page, b.src)) bad.push(`${page.path}: ${b.src}`);
    for (const f of imageFiles()) {
      const size = statSync(fileURLToPath(new URL(`../../../../docs-site/content/${f}`, import.meta.url))).size;
      if (size > 200 * 1024) bad.push(`${f}: ${Math.round(size / 1024)} KB`);
    }
    expect(bad).toEqual([]);
  });

  it("the pages the app links to exist", () => {
    for (const path of Object.values(DOC_LINKS)) if (path) expect(docs.byPath.has(path), path).toBe(true);
  });
});
