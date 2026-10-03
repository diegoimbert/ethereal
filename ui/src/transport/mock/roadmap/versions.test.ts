/** MockTransport: `Version::*` (v0.3, project-versions; CONTRACTS.md §13.11). */
import { describe, expect, it } from "vitest";
import type { RecoveryInfo, ReplyValue, VersionInfo } from "@/generated";
import { cmd } from "../../cmd";
import { createTrack, project, useMock } from "./testUtils";
import { MAX_AUTOSAVE_VERSIONS, VERSION_INTERVAL_MS, diffProjects } from "./versions";

type Reply<T extends ReplyValue["type"]> = Extract<ReplyValue, { type: T }>;

describe("MockTransport project versions (project-versions)", () => {
  const f = useMock();
  const list = async () => ((await f.mock.send(cmd("Version", { type: "List" }))) as Reply<"Versions">).versions;
  const create = async (name: string | null = null): Promise<VersionInfo> =>
    ((await f.mock.send(cmd("Version", { type: "Create", name }))) as Reply<"Version">).version;
  const recoverable = async (): Promise<RecoveryInfo[]> =>
    ((await f.mock.send(cmd("Version", { type: "ListRecoverable" }))) as Reply<"Recoverable">).projects;
  const trackCount = () => Object.keys(project(f).tracks).length;

  it("rolls autosave versions after changes, at most every interval", async () => {
    f.mock.tick(VERSION_INTERVAL_MS * 2);
    expect(await list()).toEqual([]);
    await createTrack(f, "Audio");
    f.mock.tick(VERSION_INTERVAL_MS);
    const v = await list();
    expect(v).toHaveLength(1);
    expect(v[0]).toMatchObject({ kind: "Autosave", name: null });
    expect(v[0].id).toBe(`${v[0].created_ms}-autosave`);
    expect(f.events.some((e) => e.type === "Version")).toBe(true);
    // No change since: no new version.
    f.mock.tick(VERSION_INTERVAL_MS);
    expect(await list()).toHaveLength(1);
  });

  it("keeps the newest autosave versions only", async () => {
    const manual = await create("Keep");
    for (let i = 0; i < MAX_AUTOSAVE_VERSIONS + 3; i++) {
      await createTrack(f, "Audio");
      f.mock.tick(VERSION_INTERVAL_MS);
    }
    const v = await list();
    expect(v.filter((x) => x.kind === "Autosave")).toHaveLength(MAX_AUTOSAVE_VERSIONS);
    expect(v.some((x) => x.id === manual.id)).toBe(true);
  });

  it("creates, renames, compares, restores and deletes versions", async () => {
    const before = trackCount();
    const v1 = await create("  Before drums ");
    expect(v1).toMatchObject({ kind: "Manual", name: "Before drums" });
    const v2 = await create(null);
    expect(v2.id).not.toBe(v1.id);
    await createTrack(f, "Audio");

    const diff = ((await f.mock.send(cmd("Version", { type: "Compare", version: v1.id, against: null }))) as Reply<"VersionDiff">).diff;
    expect(diff.tables.find((t) => t.table === "tracks")).toMatchObject({ added: 1, removed: 0 });
    const same = ((await f.mock.send(cmd("Version", { type: "Compare", version: v1.id, against: v2.id }))) as Reply<"VersionDiff">).diff;
    expect(same).toEqual({ tables: [], settings_changed: false });

    await f.mock.send(cmd("Version", { type: "Rename", version: v2.id, name: "Second" }));
    expect((await list()).find((x) => x.id === v2.id)?.name).toBe("Second");

    await f.mock.send(cmd("Version", { type: "Restore", version: v1.id }));
    expect(trackCount()).toBe(before);
    expect(f.events.some((e) => e.type === "Project" && e.event.type === "DirtyChanged" && e.event.dirty)).toBe(true);
    const snap = (await list()).find((x) => x.kind === "BeforeRestore")!;
    await f.mock.send(cmd("Version", { type: "Restore", version: snap.id }));
    expect(trackCount()).toBe(before + 1);

    await f.mock.send(cmd("Version", { type: "Delete", version: v2.id }));
    expect((await list()).some((x) => x.id === v2.id)).toBe(false);
    await expect(f.mock.send(cmd("Version", { type: "Delete", version: v2.id }))).rejects.toMatchObject({ code: "NotFound" });
  });

  it("offers recovery after a crash, only for unsaved, different work", async () => {
    const id = project(f).id;
    await f.mock.send(cmd("Project", { type: "Save" }));
    f.mock.tick(10);
    await createTrack(f, "Audio");
    f.mock.tick(VERSION_INTERVAL_MS);
    // A clean session leaves nothing to recover.
    expect(await recoverable()).toEqual([]);
    f.mock.versions.simulateCrash();
    const rec = await recoverable();
    expect(rec).toHaveLength(1);
    expect(rec[0]).toMatchObject({ project: id, version: { kind: "Autosave" } });
    expect(rec[0].version.created_ms).toBeGreaterThan(rec[0].saved_ms);

    const reply = (await f.mock.send(cmd("Version", { type: "Recover", project: id }))) as Reply<"Project">;
    expect(reply.project.id).toBe(id);
    expect(await recoverable()).toEqual([]);

    // Saved after the newest version: nothing to offer.
    await f.mock.send(cmd("Project", { type: "Save" }));
    f.mock.versions.simulateCrash();
    expect(await recoverable()).toEqual([]);
    await f.mock.send(cmd("Version", { type: "DiscardRecovery", project: id }));
  });

  it("diffs tables generically and lists track and clip names", () => {
    const base = project(f);
    const target = structuredClone(base);
    const tid = Object.keys(target.tracks)[0];
    target.tracks[tid] = { ...target.tracks[tid], name: "Renamed" };
    target.settings = { ...target.settings, name: "Other" };
    const d = diffProjects(base, target);
    expect(d.settings_changed).toBe(true);
    expect(d.tables).toEqual([{ table: "tracks", added: 0, removed: 0, changed: 1, names: ["Renamed"] }]);
  });
});
