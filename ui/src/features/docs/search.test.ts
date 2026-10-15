// @vitest-environment node
import { describe, expect, it } from "vitest";
import { buildDocs } from "./model";
import { buildIndex, queryTerms, search } from "./search";

const docs = buildDocs({
  "s/_section.md": "---\ntitle: basics\norder: 1\n---\n",
  "s/projects.md": [
    "---\ntitle: projects and saving\norder: 1\nsummary: Where projects are saved.\n---",
    "Intro text about saving.",
    "## deleting a project",
    "Deleting is permanent: it can't be undone.",
    "## autosave",
    "Ethereal saves as you go. Press [[key:Mod+S]] to save now.",
  ].join("\n"),
  "s/undo.md": ["---\ntitle: undo and history\norder: 2\nsummary: Go back.\n---", "## the history panel", "Every step, including deleting a track."].join("\n"),
});
const index = buildIndex(docs);

describe("docs search", () => {
  it("indexes the lead and each heading's part", () => {
    expect(index.map((e) => `${e.path}#${e.anchor}`)).toEqual([
      "s/projects#",
      "s/projects#deleting-a-project",
      "s/projects#autosave",
      "s/undo#",
      "s/undo#the-history-panel",
    ]);
    expect(index[0]!.text).toBe("Where projects are saved. Intro text about saving.");
    expect(index[2]!.text).toContain("Mod S");
  });

  it("matches word starts in titles, headings and text, headings first", () => {
    const r = search(index, "delet");
    expect(r.map((x) => `${x.entry.path}#${x.entry.anchor}`)).toEqual(["s/projects#deleting-a-project", "s/undo#the-history-panel"]);
  });

  it("needs every word, and ranks phrases", () => {
    expect(search(index, "history panel")[0]!.entry.anchor).toBe("the-history-panel");
    expect(search(index, "autosave undo")).toEqual([]);
    expect(search(index, "  ")).toEqual([]);
  });

  it("doesn't match inside words", () => {
    expect(search(index, "ave")).toEqual([]);
  });

  it("gives a snippet around the match", () => {
    const long = buildDocs({
      "s/_section.md": "---\ntitle: s\norder: 1\n---\n",
      "s/p.md": `---\ntitle: p\norder: 1\nsummary: x.\n---\n## h\n${"filler ".repeat(40)}needle here ${"tail ".repeat(40)}`,
    });
    const [hit] = search(buildIndex(long), "needle");
    expect(hit!.snippet.startsWith("…")).toBe(true);
    expect(hit!.snippet).toContain("needle here");
    expect(hit!.snippet.endsWith("…")).toBe(true);
  });

  it("splits queries into words", () => {
    expect(queryTerms("Cmd+K, the  piano-roll!")).toEqual(["cmd+k", "the", "piano-roll"]);
  });
});
