// Size caps of what a WebCLAP plugin hands the app: install downloads, ui pages.
import { describe, expect, it } from "vitest";

import { readCapped } from "./bundle";
import { MAX_UI_PAGE_FILES, collectUiPage } from "./uiPage";

function streamed(chunks: number, size: number, headers: Record<string, string> = {}): Response {
  let sent = 0;
  const body = new ReadableStream<Uint8Array>({
    pull(c) {
      if (sent++ < chunks) c.enqueue(new Uint8Array(size));
      else c.close();
    },
  });
  return new Response(body, { headers });
}

describe("readCapped", () => {
  it("reads a body within the cap", async () => {
    expect((await readCapped(streamed(4, 1000), 4000)).length).toBe(4000);
  });

  it("stops reading past the cap", async () => {
    await expect(readCapped(streamed(1000, 1000), 4000)).rejects.toThrow("too big");
  });

  it("refuses an announced size over the cap before reading", async () => {
    await expect(readCapped(streamed(1, 10, { "content-length": "5000" }), 4000)).rejects.toThrow("too big");
  });
});

describe("collectUiPage", () => {
  const file = (bytes: number) => ({ mime: "text/plain", bytes: new Uint8Array(bytes) });

  it("takes the referenced files along", async () => {
    const html = `<script src="a.js"></script><img src="/img/b.png">`;
    const page = await collectUiPage("/ui/index.html", "", async (p) =>
      p === "/ui/index.html" ? { mime: "text/html", bytes: new TextEncoder().encode(html) } : file(3),
    );
    expect(Object.keys(page.files).sort()).toEqual(["/img/b.png", "/ui/a.js"]);
  });

  it("refuses a page referencing too many files", async () => {
    const html = Array.from({ length: MAX_UI_PAGE_FILES + 1 }, (_, i) => `<img src="f${i}.png">`).join("");
    const load = async (p: string) => (p === "/index.html" ? { mime: "text/html", bytes: new TextEncoder().encode(html) } : file(1));
    await expect(collectUiPage("/index.html", "", load)).rejects.toThrow("more than");
  });

  it("refuses a page too big in all", async () => {
    const html = `<img src="a.png"><img src="b.png">`;
    const load = async (p: string) => (p === "/index.html" ? { mime: "text/html", bytes: new TextEncoder().encode(html) } : file(80 * 1024 * 1024));
    await expect(collectUiPage("/index.html", "", load)).rejects.toThrow("too big");
  });
});
