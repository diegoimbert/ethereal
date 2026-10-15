// @vitest-environment node
import { describe, expect, it } from "vitest";
import { blocksText, parseBlocks, parseInline, slugify, splitFrontMatter } from "./markdown";
import { buildDocs, neighbours } from "./model";
import { docsHref, docsLinkTarget, isDocsPage, parseDocsUrl } from "./route";

describe("front-matter", () => {
  it("reads scalars, lists and titles with colons", () => {
    const { data, body } = splitFrontMatter("---\ntitle: audio clips: fades\norder: 9\ncovers:\n  - a/b #c\n  - d/e #f\nempty:\n---\nbody\n");
    expect(data).toEqual({ title: "audio clips: fades", order: "9", covers: ["a/b #c", "d/e #f"], empty: [] });
    expect(body).toBe("body\n");
  });

  it("without front-matter, all is body", () => {
    expect(splitFrontMatter("hello")).toEqual({ data: {}, body: "hello" });
  });
});

describe("slugify", () => {
  it("lowercases, drops punctuation and joins words with -", () => {
    expect(slugify("Space and Shift+Space")).toBe("space-and-shiftspace");
    expect(slugify("what's  missing — on mobile?")).toBe("whats-missing-on-mobile");
    expect(slugify("café ×2 / ÷2")).toBe("cafe-2-2");
  });
});

describe("inline", () => {
  it("parses emphasis, code, links and the extensions", () => {
    expect(parseInline("a **b *c* d** `d` [e](/docs/x) [[action:edit.undo]] [[key:Mod+Shift]] [[desktop]]")).toEqual([
      { t: "text", v: "a " },
      {
        t: "strong",
        c: [
          { t: "text", v: "b " },
          { t: "em", c: [{ t: "text", v: "c" }] },
          { t: "text", v: " d" },
        ],
      },
      { t: "text", v: " " },
      { t: "code", v: "d" },
      { t: "text", v: " " },
      { t: "link", href: "/docs/x", c: [{ t: "text", v: "e" }] },
      { t: "text", v: " " },
      { t: "action", id: "edit.undo" },
      { t: "text", v: " " },
      { t: "key", chord: "Mod+Shift" },
      { t: "text", v: " " },
      { t: "badge", platform: "desktop" },
    ]);
  });

  it("keeps escaped characters and colons in action ids", () => {
    expect(parseInline("2 \\* 3 [[action:ai:ask]]")).toEqual([
      { t: "text", v: "2 * 3 " },
      { t: "action", id: "ai:ask" },
    ]);
  });

  it("rejects unknown extensions", () => {
    expect(() => parseInline("[[nope]]")).toThrow(/unknown extension/);
  });
});

describe("blocks", () => {
  it("parses headings with ids (explicit, deduplicated, badges left out)", () => {
    const b = parseBlocks("## recording [[desktop]]\n### same\n### same\n## custom {#my-id}\n");
    expect(b.map((x) => (x.t === "heading" ? `${x.level}:${x.id}` : x.t))).toEqual(["2:recording", "3:same", "3:same-2", "2:my-id"]);
  });

  it("parses paragraphs, nested lists, callouts, tables, code and images", () => {
    const md = [
      "first line",
      "second line",
      "",
      "- one",
      "  continued",
      "  - nested",
      "- two",
      "",
      "1. a",
      "2. b",
      "",
      "> [!warning]",
      "> careful",
      "",
      "| a | b |",
      "|---|---|",
      "| 1 | 2 \\| 3 |",
      "",
      "```",
      "code **here**",
      "```",
      '![alt](images/x.png "caption")',
      "[[shortcuts]]",
    ].join("\n");
    const b = parseBlocks(md);
    expect(b.map((x) => x.t)).toEqual(["p", "list", "list", "callout", "table", "code", "image", "shortcuts"]);
    expect(b[0]).toEqual({ t: "p", c: [{ t: "text", v: "first line second line" }] });
    const list = b[1] as Extract<(typeof b)[number], { t: "list" }>;
    expect(list.ordered).toBe(false);
    expect(list.items).toHaveLength(2);
    expect(list.items[0]!.map((x) => x.t)).toEqual(["p", "list"]);
    expect(blocksText(list.items[0]!)).toBe("one continued\nnested");
    expect((b[2] as { ordered: boolean }).ordered).toBe(true);
    expect(b[3]).toMatchObject({ t: "callout", kind: "warning", blocks: [{ t: "p" }] });
    expect(blocksText([b[4]!])).toBe("a b\n1 2 | 3");
    expect(b[5]).toEqual({ t: "code", v: "code **here**" });
    expect(b[6]).toEqual({ t: "image", alt: "alt", src: "images/x.png", caption: "caption" });
  });

  it("a list ends at a blank line followed by a paragraph", () => {
    expect(parseBlocks("- a\n\ntext").map((x) => x.t)).toEqual(["list", "p"]);
  });

  it("a plain quote is a note", () => {
    expect(parseBlocks("> hi")).toMatchObject([{ t: "callout", kind: "note" }]);
  });
});

describe("docs model", () => {
  const files = {
    "b/_section.md": "---\ntitle: b\norder: 2\n---\n",
    "a/_section.md": "---\ntitle: a\norder: 1\nsummary: first.\n---\n",
    "a/two.md": "---\ntitle: two\norder: 20\nsummary: s.\nplatform: desktop, web\n---\n## h\n",
    "a/one.md": "---\ntitle: one\norder: 10\nsummary: s.\naudience: beginner\n---\n",
    "b/three.md": "---\ntitle: three\norder: 1\nsummary: s.\n---\n",
  };

  it("orders sections and pages, and links neighbours across sections", () => {
    const docs = buildDocs(files);
    expect(docs.sections.map((s) => s.slug)).toEqual(["a", "b"]);
    expect(docs.pages.map((p) => p.path)).toEqual(["a/one", "a/two", "b/three"]);
    expect(docs.byPath.get("a/two")).toMatchObject({ platforms: ["desktop", "web"], audience: "everyone", headings: [{ id: "h" }] });
    expect(neighbours(docs, "a/two")).toMatchObject({ prev: { path: "a/one" }, next: { path: "b/three" } });
    expect(neighbours(docs, "a/one").prev).toBeNull();
  });

  it("names the file of invalid content", () => {
    expect(() => buildDocs({ ...files, "a/bad.md": "---\norder: 1\n---\n" })).toThrow(/a\/bad\.md: front-matter "title" is required/);
    expect(() => buildDocs({ ...files, "a/bad.md": "---\ntitle: x\norder: 1\nsummary: s.\nplatform: tv\n---\n" })).toThrow(/platform/);
    expect(() => buildDocs({ ...files, "a/Bad Name.md": "" })).toThrow(/lowercase/);
    expect(() => buildDocs({ "c/x.md": "---\ntitle: x\norder: 1\nsummary: s.\n---\n" })).toThrow(/_section\.md is missing/);
  });
});

describe("docs urls", () => {
  it("routes /docs paths and builds them back", () => {
    expect(isDocsPage("/docs")).toBe(true);
    expect(isDocsPage("/docs/a/b")).toBe(true);
    expect(isDocsPage("/docsx")).toBe(false);
    expect(isDocsPage("/p/x/y")).toBe(false);
    expect(parseDocsUrl("/docs/Basics/Projects/extra", "#the-dot")).toEqual({ path: "basics/projects", anchor: "the-dot" });
    expect(parseDocsUrl("/docs/")).toEqual({ path: "", anchor: "" });
    expect(docsHref("a/b", "c")).toBe("/docs/a/b#c");
    expect(docsHref("")).toBe("/docs");
    expect(docsLinkTarget("/docs/a/b#c")).toEqual({ path: "a/b", anchor: "c" });
    expect(docsLinkTarget("https://x.y")).toBeNull();
    expect(docsLinkTarget("/download")).toBeNull();
  });
});
