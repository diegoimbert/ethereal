// The docs as data: sections and pages read from Markdown files (`docs-site/content`,
// docs-site/README.md). Pure: `content.ts` feeds it the bundled files, tests feed it fixtures.
import { parseBlocks, PLATFORMS, splitFrontMatter, type Block, type FrontMatterValue, type Inline, type Platform } from "./markdown";

export type Audience = "beginner" | "everyone" | "advanced";

export interface DocHeading {
  id: string;
  level: 2 | 3 | 4;
  c: Inline[];
}

export interface Coverage {
  /** Inventory item id (`<area>/<slug>`, docs-site/inventory.json). */
  item: string;
  /** Heading id on the page. */
  anchor: string;
}

export interface DocPage {
  /** `<section>/<page>`: the URL is `/docs/<path>`. */
  path: string;
  section: string;
  slug: string;
  title: string;
  order: number;
  summary: string;
  /** Empty: every platform. */
  platforms: Platform[];
  audience: Audience;
  covers: Coverage[];
  blocks: Block[];
  headings: DocHeading[];
  /** The source file (relative to docs-site/content), for errors. */
  file: string;
}

export interface DocSection {
  slug: string;
  title: string;
  order: number;
  summary: string;
  pages: DocPage[];
}

export interface Docs {
  sections: DocSection[];
  /** Every page, in reading order (sections, then pages). */
  pages: DocPage[];
  byPath: ReadonlyMap<string, DocPage>;
}

const AUDIENCES: ReadonlyArray<Audience> = ["beginner", "everyone", "advanced"];

/**
 * Builds the docs from `{ "<section>/<page>.md": source }` (paths relative to the content
 * root; `<section>/_section.md` holds the section's title and order). Throws on invalid
 * content, naming the file.
 */
export function buildDocs(files: Readonly<Record<string, string>>): Docs {
  const sections = new Map<string, DocSection>();
  const pages: DocPage[] = [];
  for (const [file, src] of Object.entries(files)) {
    const m = /^([a-z0-9-]+)\/([a-z0-9_-]+)\.md$/.exec(file);
    if (!m) throw new Error(`docs: ${file}: pages are <section>/<page>.md, lowercase with hyphens`);
    const [, section, slug] = m as unknown as [string, string, string];
    try {
      const { data, body } = splitFrontMatter(src);
      if (slug === "_section") {
        const s = sectionOf(sections, section);
        s.title = str(data, "title");
        s.order = num(data, "order");
        s.summary = str(data, "summary", "");
        continue;
      }
      const blocks = parseBlocks(body);
      pages.push({
        path: `${section}/${slug}`,
        section,
        slug,
        title: str(data, "title"),
        order: num(data, "order"),
        summary: str(data, "summary"),
        platforms: platformsOf(str(data, "platform", "all")),
        audience: oneOf(str(data, "audience", "everyone"), AUDIENCES, "audience"),
        covers: coversOf(data.covers),
        blocks,
        headings: headingsOf(blocks),
        file,
      });
    } catch (e) {
      throw new Error(`docs: ${file}: ${e instanceof Error ? e.message : String(e)}`, { cause: e });
    }
  }
  for (const p of pages) sectionOf(sections, p.section).pages.push(p);
  const list = [...sections.values()].filter((s) => s.pages.length > 0);
  for (const s of list) {
    if (!s.title) throw new Error(`docs: ${s.slug}/_section.md is missing (title, order)`);
    s.pages.sort((a, b) => a.order - b.order || a.slug.localeCompare(b.slug));
  }
  list.sort((a, b) => a.order - b.order || a.slug.localeCompare(b.slug));
  const ordered = list.flatMap((s) => s.pages);
  return { sections: list, pages: ordered, byPath: new Map(ordered.map((p) => [p.path, p])) };
}

function sectionOf(map: Map<string, DocSection>, slug: string): DocSection {
  let s = map.get(slug);
  if (!s) map.set(slug, (s = { slug, title: "", order: 0, summary: "", pages: [] }));
  return s;
}

function str(data: Record<string, FrontMatterValue>, key: string, fallback?: string): string {
  const v = data[key];
  if (typeof v === "string" && v) return v;
  if (fallback !== undefined && v === undefined) return fallback;
  throw new Error(`front-matter "${key}" is required`);
}

function num(data: Record<string, FrontMatterValue>, key: string): number {
  const n = Number(str(data, key));
  if (!Number.isFinite(n)) throw new Error(`front-matter "${key}" must be a number`);
  return n;
}

function oneOf<T extends string>(v: string, allowed: ReadonlyArray<T>, key: string): T {
  if (!(allowed as readonly string[]).includes(v)) throw new Error(`front-matter "${key}" must be one of ${allowed.join(", ")}`);
  return v as T;
}

function platformsOf(v: string): Platform[] {
  if (v === "all") return [];
  return v.split(",").map((p) => oneOf(p.trim(), PLATFORMS, "platform"));
}

function coversOf(v: FrontMatterValue | undefined): Coverage[] {
  if (v === undefined) return [];
  if (typeof v === "string") throw new Error(`front-matter "covers" must be a list`);
  return v.map((line) => {
    const m = /^([a-z0-9-]+\/[a-z0-9-]+)\s+#([a-z0-9-]+)$/.exec(line);
    if (!m) throw new Error(`covers: "${line}" must be "<item id> #<anchor>"`);
    return { item: m[1]!, anchor: m[2]! };
  });
}

function headingsOf(blocks: ReadonlyArray<Block>): DocHeading[] {
  const out: DocHeading[] = [];
  const walk = (list: ReadonlyArray<Block>) => {
    for (const b of list) {
      if (b.t === "heading") out.push({ id: b.id, level: b.level, c: b.c });
      else if (b.t === "callout") walk(b.blocks);
      else if (b.t === "list") b.items.forEach(walk);
    }
  };
  walk(blocks);
  return out;
}

/** The previous and next pages in reading order. */
export function neighbours(docs: Docs, path: string): { prev: DocPage | null; next: DocPage | null } {
  const i = docs.pages.findIndex((p) => p.path === path);
  if (i < 0) return { prev: null, next: null };
  return { prev: docs.pages[i - 1] ?? null, next: docs.pages[i + 1] ?? null };
}

/** What a location shows. */
export type DocsView = { kind: "home" } | { kind: "page"; page: DocPage } | { kind: "missing"; path: string };

export function resolveView(docs: Docs, path: string): DocsView {
  if (!path) return { kind: "home" };
  const page = docs.byPath.get(path) ?? docs.sections.find((s) => s.slug === path)?.pages[0];
  return page ? { kind: "page", page } : { kind: "missing", path };
}
