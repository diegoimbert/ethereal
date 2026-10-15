import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { MockTransport } from "../MockTransport";
import { compileFileFilter, defaultFileFilter } from "./fileFilter";

const names = async (t: MockTransport, path: string) => {
  const r = await t.send(cmd("Media", { type: "ListDirectory", location: { type: "Library", id: "library" }, path }));
  if (r.type !== "Directory") throw new Error(r.type);
  return r.listing.entries.map((e) => e.name);
};

describe("mock file filters (base-145)", () => {
  it("hides unsupported files from listings by default; Set shows them and emits", async () => {
    const t = new MockTransport({ timers: "manual", seed: 1 });
    expect(await names(t, "Drums")).toEqual(["Loops", "Hat Closed.wav", "Kick.wav", "Snare.wav"]);
    const events: string[] = [];
    t.onEvent((e) => {
      if (e.type === "Browser") events.push(e.event.type);
    });
    const filter = { ...defaultFileFilter(), whitelist: [...defaultFileFilter().whitelist, String.raw`\.als$`] };
    const r = await t.send(cmd("Browser", { type: "SetFileFilter", filter }));
    expect(r).toMatchObject({ type: "BrowserFileFilter", filter });
    expect(events).toContain("FileFilterChanged");
    expect(await names(t, "Drums")).toContain("Kit.als");
    await expect(t.send(cmd("Browser", { type: "SetFileFilter", filter: { whitelist: ["("], blacklist: [] } }))).rejects.toMatchObject({
      error: { code: "InvalidArgument" },
    });
  });

  it("checks rules, paths and library counts", async () => {
    const t = new MockTransport({ timers: "manual", seed: 1 });
    const r = await t.send(cmd("Browser", { type: "CheckFileFilter", filter: null, paths: ["a/b.wav", ".git/c.wav", "x.als"] }));
    if (r.type !== "BrowserFileFilterCheck") throw new Error(r.type);
    expect(r.check.visible).toEqual([true, false, false]);
    expect(r.check.library).toEqual({ shown: 10, total: 13 });
    expect(compileFileFilter({ whitelist: ["ok", " "], blacklist: ["[z-a]"] }).errors.map((e) => [e.list, e.index])).toEqual([
      ["Whitelist", 1],
      ["Blacklist", 0],
    ]);
  });
});
