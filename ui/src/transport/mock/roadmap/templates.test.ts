/** MockTransport: `Template::*` (v0.3, templates), same rules as `crates/ether-controller/tests/templates.rs`. */
import { describe, expect, it } from "vitest";
import type { PresetMeta, ReplyValue, TemplateInfo } from "@/generated";
import { deriveId } from "@/features/comping/model";
import { cmd } from "../../cmd";
import { createTrack, project, testId, undo, useMock, type MockFixture } from "./testUtils";

const meta: PresetMeta = { tags: [], author: null, description: null };

async function list(f: MockFixture, kind: "Project" | "Tracks" | null = null): Promise<TemplateInfo[]> {
  const r = await f.mock.send(cmd("Template", { type: "List", kind }));
  if (r.type !== "Templates") throw new Error(r.type);
  return r.templates;
}

const template = (r: ReplyValue): TemplateInfo => {
  if (r.type !== "Template") throw new Error(r.type);
  return r.template;
};

describe("MockTransport templates (templates)", () => {
  const f = useMock();

  it("lists the factory templates first and saves user track templates", async () => {
    const all = await list(f);
    expect(all.length).toBeGreaterThanOrEqual(4);
    expect(all.every((t) => t.factory && t.kind === "Tracks")).toBe(true);

    const track = await createTrack(f, "Midi");
    await f.mock.send(cmd("Device", { type: "Insert", id: testId(), track, device: { type: "Builtin", device: { type: "PolySynth" } }, before: null }));
    const saved = template(await f.mock.send(cmd("Template", { type: "SaveTracks", tracks: [track], name: "Keys", meta: { ...meta, tags: ["Warm"] }, overwrite: false })));
    expect(saved).toMatchObject({ id: "tracks/Keys", kind: "Tracks", name: "Keys", factory: false, meta: { tags: ["warm"] } });
    expect(f.events).toContainEqual({ type: "Template", event: { type: "Changed" } });
    await expect(f.mock.send(cmd("Template", { type: "SaveTracks", tracks: [track], name: "keys", meta, overwrite: false }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    const user = (await list(f, "Tracks")).filter((t) => !t.factory);
    expect(user.map((t) => t.id)).toEqual(["tracks/Keys"]);
  });

  it("inserts a template as one undo step with derived ids", async () => {
    const track = await createTrack(f, "Midi");
    const dev = testId();
    await f.mock.send(cmd("Device", { type: "Insert", id: dev, track, device: { type: "Builtin", device: { type: "PolySynth" } }, before: null }));
    await f.mock.send(cmd("Template", { type: "SaveTracks", tracks: [track], name: "Keys", meta, overwrite: false }));
    const before = project(f);
    const seed = testId();
    await f.mock.send(cmd("Template", { type: "Insert", template: "tracks/Keys", seed, parent: null, before: null }));
    const after = project(f);
    const newTrack = deriveId(seed, 0);
    expect(after.tracks[newTrack]).toMatchObject({ kind: "Midi", parent: null });
    const newDevice = after.devices[deriveId(seed, 1)];
    expect(newDevice).toMatchObject({ track: newTrack, kind: { type: "Builtin", device: { type: "PolySynth" } } });
    // Retried: no change.
    await f.mock.send(cmd("Template", { type: "Insert", template: "tracks/Keys", seed, parent: null, before: null }));
    expect(project(f)).toEqual(after);
    await undo(f);
    expect(project(f)).toEqual(before);
  });

  it("inserts factory templates (groups keep their children)", async () => {
    const seed = testId();
    await f.mock.send(cmd("Template", { type: "Insert", template: "factory/drum-bus", seed, parent: null, before: null }));
    const p = project(f);
    const group = p.tracks[deriveId(seed, 0)]!;
    expect(group.kind).toBe("Group");
    const kids = Object.values(p.tracks).filter((t) => t.parent === group.id);
    expect(kids.map((t) => t.name)).toEqual(["Kit"]);
    await expect(f.mock.send(cmd("Template", { type: "Delete", template: "factory/drum-bus" }))).rejects.toMatchObject({ code: "InvalidArgument" });
  });

  it("creates projects from project templates and the default", async () => {
    const track = await createTrack(f, "Audio");
    await f.mock.send(cmd("Track", { type: "Rename", id: track, name: "Guitar" }));
    await f.mock.send(cmd("Template", { type: "SaveProject", name: "Band", meta, overwrite: false }));
    const id = testId();
    const r = await f.mock.send(cmd("Template", { type: "NewProject", id, name: "Song", template: "projects/Band" }));
    expect(r.type).toBe("Project");
    expect(project(f).id).toBe(id);
    expect(project(f).settings.name).toBe("Song");
    expect(Object.values(project(f).tracks).map((t) => t.name)).toContain("Guitar");

    // No default: empty.
    await f.mock.send(cmd("Template", { type: "NewProject", id: testId(), name: "Empty", template: null }));
    expect(Object.keys(project(f).tracks)).toHaveLength(1);

    await f.mock.send(cmd("Template", { type: "SetDefault", template: "projects/Band" }));
    expect((await list(f, "Project")).find((t) => t.id === "projects/Band")?.default).toBe(true);
    await f.mock.send(cmd("Template", { type: "NewProject", id: testId(), name: "Default", template: null }));
    expect(Object.values(project(f).tracks).map((t) => t.name)).toContain("Guitar");

    // Rename keeps the default, delete clears it.
    const renamed = template(await f.mock.send(cmd("Template", { type: "Rename", template: "projects/Band", name: "Band 2" })));
    expect(renamed).toMatchObject({ id: "projects/Band 2", default: true });
    await f.mock.send(cmd("Template", { type: "Delete", template: "projects/Band 2" }));
    await f.mock.send(cmd("Template", { type: "NewProject", id: testId(), name: "After", template: null }));
    expect(Object.keys(project(f).tracks)).toHaveLength(1);
  });
});
