/** MockTransport: `Browser` (v0.2, browser-v2). */
import { describe, expect, it } from "vitest";
import type { BrowserCommand, BrowserQuery, Command, Event, LibraryItem, ReplyValue } from "@/generated";
import type { MockHost } from "./host";
import { MockBrowser, parseNameMeta } from "./browserV2";

function fixture() {
  const events: Event[] = [];
  const executed: Command[] = [];
  const host = {
    emit: (e: Event) => void events.push(e),
    execute: (c: Command) => void executed.push(c),
  } as unknown as MockHost;
  const browser = new MockBrowser(host);
  return { browser, events, executed };
}

const q = (over: Partial<BrowserQuery> = {}): BrowserQuery => ({
  text: "",
  kinds: [],
  tags: [],
  favourites_only: false,
  roots: [],
  folder: null,
  device: null,
  sort: "Name",
  offset: 0,
  limit: 100,
  ...over,
});

function page(b: MockBrowser, over: Partial<BrowserQuery> = {}) {
  const r = b.command({ type: "Query", query: q(over) });
  if (r.type !== "BrowserPage") throw new Error(r.type);
  return r.page;
}
const names = (items: LibraryItem[]) => items.map((i) => i.name);
const run = (b: MockBrowser, c: BrowserCommand): ReplyValue => b.command(c);

describe("MockBrowser (browser-v2)", () => {
  it("parses bpm and key from file names", () => {
    expect(parseNameMeta("Break 120.wav", "Drums/Loops/Break 120.wav")).toEqual({ bpm: 120, key: null });
    expect(parseNameMeta("Bass_Am_128bpm.wav", "Bass/Bass_Am_128bpm.wav")).toEqual({ bpm: 128, key: "A minor" });
    expect(parseNameMeta("Pad C#maj.wav", "Pads/Pad C#maj.wav")).toEqual({ bpm: null, key: "C# major" });
    expect(parseNameMeta("Kick 100.wav", "Drums/Kick 100.wav")).toEqual({ bpm: null, key: null });
    expect(parseNameMeta("Keys Bbm 90.wav", "Keys/Keys Bbm 90.wav")).toEqual({ bpm: 90, key: "Bb minor" });
  });

  it("lists roots with item counts (library, packs, user library, factory)", () => {
    const { browser } = fixture();
    const r = run(browser, { type: "ListRoots" });
    if (r.type !== "BrowserRoots") throw new Error(r.type);
    const byId = Object.fromEntries(r.roots.map((x) => [x.id, x]));
    expect(byId.library).toMatchObject({ kind: "Library", items: 10 });
    expect(byId["library/Vocals"]).toMatchObject({ kind: "Pack", name: "Vocals", items: 2 });
    expect(byId.user).toMatchObject({ kind: "Library", items: 0 });
    expect(byId.factory!.kind).toBe("Factory");
    expect(byId.factory!.items).toBeGreaterThan(0);
  });

  it("indexes audio with metadata and presets with refs", () => {
    const { browser } = fixture();
    const [brk] = page(browser, { text: "break" }).items;
    expect(brk).toMatchObject({
      id: "library/Drums/Loops/Break 120.wav",
      kind: "Audio",
      source: { type: "Location", location: { type: "Library", id: "library" }, path: "Drums/Loops/Break 120.wav" },
      meta: { bpm: 120, duration_seconds: 8, sample_rate: 44100, channels: 2, pack: "Library" },
    });
    const preset = page(browser, { kinds: ["Preset"], limit: 1 }).items[0]!;
    expect(preset.root).toBe("factory");
    expect(preset.preset).toEqual({ source: "Factory", id: preset.path });
    expect(page(browser, { text: "chop" }).items[0]!.meta.pack).toBe("Vocals");
  });

  it("ANDs text words, kinds, roots and folders", () => {
    const { browser } = fixture();
    expect(names(page(browser, { text: "drums loop" }).items)).toEqual(["Break 120.wav", "Shuffle 96.wav"]);
    expect(names(page(browser, { text: "groove", kinds: ["Audio"] }).items)).toEqual([]);
    expect(names(page(browser, { kinds: ["Midi"] }).items)).toEqual(["Groove 1.mid"]);
    expect(names(page(browser, { roots: ["library/Vocals"] }).items)).toEqual(["Chop 1.wav", "Phrase 2.wav"]);
    expect(names(page(browser, { roots: ["library"], folder: "Drums/Loops" }).items)).toEqual(["Break 120.wav", "Shuffle 96.wav"]);
    expect(page(browser, { roots: ["factory"] }).items.every((i) => i.kind === "Preset")).toBe(true);
    // Presets restricted to a device; other kinds unaffected.
    const forComp = page(browser, { device: { type: "Builtin", device: "Compressor" } }).items;
    expect(forComp.filter((i) => i.kind === "Preset").every((i) => i.path.startsWith("compressor/"))).toBe(true);
    expect(forComp.some((i) => i.kind === "Audio")).toBe(true);
  });

  it("sorts by relevance, duration, bpm and recency (nulls last) and pages", () => {
    const { browser } = fixture();
    expect(names(page(browser, { text: "pad", sort: "Relevance", roots: ["library"] }).items)).toEqual(["Pad C.wav"]);
    const rel = names(page(browser, { text: "s", sort: "Relevance", roots: ["library"] }).items);
    expect(rel[0]).toBe("Shuffle 96.wav"); // name prefix before "contains"
    const dur = page(browser, { sort: "Duration", roots: ["library"] }).items;
    expect(dur[0]!.name).toBe("Hat Closed.wav");
    expect(dur.at(-1)!.meta.duration_seconds).toBeNull();
    const bpm = page(browser, { sort: "Bpm", roots: ["library"] }).items;
    expect(names(bpm.slice(0, 2))).toEqual(["Shuffle 96.wav", "Break 120.wav"]);
    expect(page(browser, { sort: "Recent", roots: ["library"] }).items[0]!.name).toBe("Kick.wav");

    const p1 = page(browser, { roots: ["library"], limit: 3 });
    const p2 = page(browser, { roots: ["library"], limit: 3, offset: 3 });
    expect(p1).toMatchObject({ total: 10, offset: 0 });
    expect(p1.items).toHaveLength(3);
    expect(p2.offset).toBe(3);
    expect(names(p2.items)).not.toContain(p1.items[0]!.name);
    expect(page(browser, { limit: 0 }).items).toHaveLength(1);
    expect(page(browser, { limit: 10_000 }).items.length).toBeLessThanOrEqual(200);
  });

  it("stores favourites and normalized tags, emitting IndexChanged", () => {
    const { browser, events } = fixture();
    const id = "library/Drums/Kick.wav";
    expect(run(browser, { type: "SetFavourite", item: id, favourite: true })).toEqual({ type: "Unit" });
    expect(names(page(browser, { favourites_only: true }).items)).toEqual(["Kick.wav"]);
    run(browser, { type: "SetTags", item: id, tags: [" Punchy", "acoustic", "punchy", ""] });
    expect(page(browser, { tags: ["punchy"] }).items).toMatchObject([{ id, tags: ["acoustic", "punchy"], favourite: true }]);
    expect(names(page(browser, { text: "acoustic" }).items)).toEqual(["Kick.wav"]);
    run(browser, { type: "SetFavourite", item: id, favourite: false });
    expect(page(browser, { favourites_only: true }).total).toBe(0);
    expect(events.filter((e) => e.type === "Browser" && e.event.type === "IndexChanged")).toHaveLength(3);
    expect(() => run(browser, { type: "SetTags", item: "library/nope.wav", tags: [] })).toThrow(expect.objectContaining({ code: "NotFound" }));
  });

  it("previews audio through Media::Preview and rejects other kinds", () => {
    const { browser, executed } = fixture();
    run(browser, { type: "Preview", item: "library/Drums/Kick.wav", sync: true });
    expect(executed).toEqual([
      {
        domain: "Media",
        command: { type: "Preview", source: { type: "Location", location: { type: "Library", id: "library" }, path: "Drums/Kick.wav" } },
      },
    ]);
    expect(() => run(browser, { type: "Preview", item: "library/MIDI/Groove 1.mid", sync: false })).toThrow(
      expect.objectContaining({ code: "InvalidArgument" }),
    );
    expect(() => new MockBrowser(null).command({ type: "Preview", item: "library/Drums/Kick.wav", sync: false })).toThrow(
      expect.objectContaining({ code: "Unsupported" }),
    );
  });

  it("rescans with progress, refuses AddFolder on the web and non-folder removal", () => {
    const { browser, events } = fixture();
    run(browser, { type: "Rescan", root: "library" });
    expect(events).toEqual([
      { type: "Browser", event: { type: "IndexProgress", root: "library", scanned: 10, total: 10 } },
      { type: "Browser", event: { type: "IndexChanged" } },
    ]);
    expect(() => run(browser, { type: "Rescan", root: "nope" })).toThrow(expect.objectContaining({ code: "NotFound" }));
    expect(() => run(browser, { type: "AddFolder", path: "/x" })).toThrow(expect.objectContaining({ code: "Unsupported" }));
    expect(() => run(browser, { type: "RemoveFolder", root: "library" })).toThrow(expect.objectContaining({ code: "InvalidArgument" }));
  });
});
