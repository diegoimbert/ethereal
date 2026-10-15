// @vitest-environment node
// Coverage of the explain inventory by the user docs (#665): every item that still applies has
// a page and a heading (`covers:`), and docs-site/coverage.json is the current report.
// After changing pages, `pending.txt` or the inventory, refresh the report with
//   ETHER_DOCS_UPDATE=1 pnpm --filter @ethereal/ui exec vitest run src/features/docs/coverage.test.ts
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { getDocs } from "./content";
import { checkCoverage, parsePending, type Inventory } from "./coverage";
import { buildDocs } from "./model";

const site = (f: string) => fileURLToPath(new URL(`../../../../docs-site/${f}`, import.meta.url));
const inventory = JSON.parse(readFileSync(site("inventory.json"), "utf8")) as Inventory;
const pendingFile = parsePending(readFileSync(site("pending.txt"), "utf8"));
const pending = pendingFile.items;

describe("docs coverage of the explain inventory", () => {
  const { report, problems } = checkCoverage(getDocs(), inventory, pending);

  it("every covers: line names a live inventory item and a heading on its page", () => {
    expect(problems.unknownItems, "unknown or removed inventory ids in covers:").toEqual([]);
    expect(problems.missingAnchors, "anchors that aren't a heading id on the page").toEqual([]);
  });

  it("every item that still applies has a page (or is pending)", () => {
    expect(problems.uncovered, "inventory items with no page: cover them, or mark them removed with a reason").toEqual([]);
    expect(problems.removedWithoutReason).toEqual([]);
  });

  it("pending.txt lists only items still to write", () => {
    expect(problems.stalePending, "remove these from docs-site/pending.txt").toEqual([]);
  });

  it("docs-site/coverage.json is up to date", () => {
    const text = `${JSON.stringify(report, null, 2)}\n`;
    if (process.env.ETHER_DOCS_UPDATE === "1") writeFileSync(site("coverage.json"), text);
    expect(readFileSync(site("coverage.json"), "utf8"), "run with ETHER_DOCS_UPDATE=1 to refresh coverage.json").toBe(text);
  });

  it("pending pages aren't written yet", () => {
    const written = pendingFile.pages.filter((p) => getDocs().byPath.has(p));
    expect(written, "remove these `page …` lines from docs-site/pending.txt").toEqual([]);
  });

  it("inventory ids are unique and area-prefixed", () => {
    const ids = inventory.areas.flatMap((a) => a.items.map((i) => i.id));
    expect(new Set(ids).size).toBe(ids.length);
    for (const a of inventory.areas) for (const i of a.items) expect(i.id.startsWith(`${a.id}/`), i.id).toBe(true);
  });
});

describe("parsePending", () => {
  it("reads item ids and pending pages, skipping comments", () => {
    expect(parsePending("# note\na/one\npage basics/x # later\n\n b/two \n")).toEqual({ items: ["a/one", "b/two"], pages: ["basics/x"] });
  });
});

describe("checkCoverage", () => {
  const docs = buildDocs({
    "s/_section.md": "---\ntitle: s\norder: 1\n---\n",
    "s/p.md": "---\ntitle: p\norder: 1\nsummary: x.\ncovers:\n  - a/one #first\n  - a/two #nope\n---\n## first\n",
  });
  const inv: Inventory = {
    areas: [
      {
        id: "a",
        name: "a",
        items: [
          { id: "a/one", title: "one" },
          { id: "a/two", title: "two" },
          { id: "a/three", title: "three" },
          { id: "a/four", title: "four", removed: "gone in #1" },
          { id: "a/five", title: "five" },
        ],
      },
    ],
  };

  it("reports missing anchors, uncovered, stale pending and the percentage", () => {
    const { report, problems } = checkCoverage(docs, inv, ["a/five", "a/one"]);
    expect(problems.missingAnchors).toEqual(["s/p: #nope (for a/two)"]);
    expect(problems.uncovered).toEqual(["a/three"]);
    expect(problems.stalePending).toEqual(["a/one"]);
    expect(report).toMatchObject({ total: 5, removed: 1, covered: 2, percent: 50, removedItems: { "a/four": "gone in #1" } });
    expect(report.items["a/one"]).toEqual(["s/p#first"]);
  });

  it("flags covers of removed or unknown items", () => {
    const d = buildDocs({
      "s/_section.md": "---\ntitle: s\norder: 1\n---\n",
      "s/p.md": "---\ntitle: p\norder: 1\nsummary: x.\ncovers:\n  - a/four #h\n  - a/zzz #h\n---\n## h\n",
    });
    expect(checkCoverage(d, inv, []).problems.unknownItems).toEqual(["s/p: a/four (removed)", "s/p: a/zzz"]);
  });
});
