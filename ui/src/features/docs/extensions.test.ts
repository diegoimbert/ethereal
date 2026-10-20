// @vitest-environment node
// Pages a build adds (`extendDocs`): they join the bundled docs, with their translations and images.
import { describe, expect, it } from "vitest";
import { contentFiles, getDocs, imageUrl, loadDocs, staleTranslations } from "./content";
import { extendDocs, extendDocsLazily, loadDocsExtensions } from "./extensions";

const page = (title: string, heading: string) =>
  `---\ntitle: ${title}\norder: 1\nsummary: A page.\n---\n\n![x](images/x.png)\n\n## ${heading}\n\n## where next\n\n- [first](/docs/welcome)\n`;

describe("extendDocs", () => {
  const before = getDocs("en").pages.length;
  extendDocs({
    pages: { "extra/_section.md": "---\ntitle: extra\norder: 99\n---\n", "extra/thing.md": page("a thing", "using it") },
    translations: { fr: { "extra/_section.md": "---\ntitle: en plus\norder: 99\n---\n", "extra/thing.md": page("une chose", "l’utiliser {#using-it}") } },
    images: { "extra/images/x.png": "/assets/x.png" },
  });

  it("adds the pages and their section to the docs", () => {
    const docs = getDocs("en");
    expect(docs.pages.length).toBe(before + 1);
    expect(docs.byPath.get("extra/thing")?.title).toBe("a thing");
    expect(docs.sections.at(-1)?.slug).toBe("extra");
    expect(contentFiles()["extra/thing.md"]).toBeDefined();
  });

  it("resolves the pages' images", () => {
    expect(imageUrl(getDocs("en").byPath.get("extra/thing")!, "images/x.png")).toBe("/assets/x.png");
  });

  it("loads their translations with the language's", async () => {
    await loadDocs("fr");
    expect(getDocs("fr").byPath.get("extra/thing")?.title).toBe("une chose");
    expect(staleTranslations({ "extra/thing.md": page("une chose", "l’utiliser {#using-it}") })).toEqual([]);
  });
});

describe("extendDocsLazily", () => {
  it("loads the pages when the docs ask for them, once", async () => {
    let loads = 0;
    extendDocsLazily(async () => {
      loads += 1;
      return { pages: { "extra/later.md": page("later", "eventually") } };
    });
    expect(loads).toBe(0);
    expect(getDocs("en").byPath.get("extra/later")).toBeUndefined();
    await loadDocsExtensions();
    await loadDocsExtensions();
    expect(loads).toBe(1);
    expect(getDocs("en").byPath.get("extra/later")?.title).toBe("later");
  });
});
