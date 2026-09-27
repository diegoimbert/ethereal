/** groups-buses UI: pure helpers, Cmd+G / ungroup in the arrangement, VCAs and inputs in the mixer. */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Project, Track, TrackId } from "@/generated";
import { ContextMenuHost } from "@/kit";
import { pickOption } from "@/kit/testing";
import { useProjectStore, useSelectionStore } from "@/state";
import { cmd, createDemoProject, MockTransport, newId, TransportProvider } from "@/transport";
import { ArrangementView } from "@/features/arrangement";
import { resetArrangementUi } from "@/features/arrangement/uiStore";
import { resetAutomationUi } from "@/features/automation";
import { Mixer } from "@/features/mixer";
import { groupCommand, groupShortcut, inputSources, ungroupLosses, vcaTargets } from "./index";
import { childrenOf } from "./model";

const store = () => useProjectStore.getState();
const project = () => store().project!;
const byName = (name: string): Track => {
  const t = Object.values(project().tracks).find((x) => x.name === name);
  if (!t) throw new Error(`no track ${name}`);
  return t;
};
const names = (parent: TrackId | null) => childrenOf(project().tracks, parent).map((t) => t.name);

let mock: MockTransport | undefined;

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function renderApp(ui: "arrangement" | "mixer") {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  // Clip canvases need a 2D context.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
    () => new Proxy({}, { get: () => () => {} }) as unknown as CanvasRenderingContext2D,
  );
  render(
    <TransportProvider transport={mock}>
      {ui === "arrangement" ? <ArrangementView /> : <Mixer />}
      <ContextMenuHost />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  await flush();
  return mock;
}

afterEach(() => {
  mock?.dispose();
  mock = undefined;
  store().reset();
  useSelectionStore.getState().selectTrack(null);
  resetArrangementUi();
  resetAutomationUi();
  vi.restoreAllMocks();
});

describe("groups model", () => {
  const demo = (): Project => createDemoProject(1);
  const find = (p: Project, name: string) => Object.values(p.tracks).find((t) => t.name === name)!;

  it("groups siblings directly and gathers strays with a Batch", () => {
    const p = demo();
    const keys = find(p, "Keys");
    const bass = find(p, "Bass");
    expect(groupCommand(p.tracks, [bass.id, keys.id], "G")).toEqual(
      cmd("Track", { type: "GroupSelected", ids: [keys.id, bass.id], group: "G", name: null }),
    );
    // Bass inside a group, Keys at the top: Bass moves next to Keys first (one step).
    p.tracks["g0"] = { ...keys, id: "g0", kind: "Group", name: "Old", order: "zzz" };
    p.tracks[bass.id] = { ...bass, parent: "g0" };
    const c = groupCommand(p.tracks, [keys.id, bass.id], "G");
    expect(c?.domain).toBe("Edit");
    const batch = c?.command as { type: string; commands: { command: { type: string; id?: string; parent?: string | null } }[] };
    expect(batch.type).toBe("Batch");
    expect(batch.commands.map((x) => x.command.type)).toEqual(["Move", "GroupSelected"]);
    expect(batch.commands[0]!.command).toMatchObject({ id: bass.id, parent: null });
    // Master/returns alone: nothing to group.
    expect(groupCommand(p.tracks, [find(p, "Master").id, find(p, "A Delay").id], "G")).toBeNull();
  });

  it("keeps routing choices cycle-free", () => {
    const p = demo();
    const keys = find(p, "Keys");
    const drums = find(p, "Drums");
    p.tracks[keys.id] = { ...keys, input: { type: "Track", track: drums.id, tap: "PostFader" } };
    // Drums feeds Keys: Keys is not a source for Drums; master and Drums itself neither.
    const sources = inputSources(p.tracks, p.tracks[drums.id]!).map((t) => t.name);
    expect(sources).not.toContain("Keys");
    expect(sources).not.toContain("Drums");
    expect(sources).not.toContain("Master");
    expect(sources).toContain("Bass");
    p.tracks["v1"] = { ...keys, id: "v1", kind: "Vca", name: "V1", input: { type: "None" } };
    p.tracks["v2"] = { ...keys, id: "v2", kind: "Vca", name: "V2", input: { type: "None" }, vca: "v1" };
    expect(vcaTargets(p.tracks, p.tracks["v1"]!).map((t) => t.id)).toEqual([]);
    expect(vcaTargets(p.tracks, p.tracks["v2"]!).map((t) => t.id)).toEqual(["v1"]);
    expect(vcaTargets(p.tracks, find(p, "Master"))).toEqual([]);
  });

  it("lists what ungrouping loses", () => {
    const p = demo();
    expect(ungroupLosses(p, find(p, "Keys").id)).toEqual(["2 devices", "1 automation lane", "1 send"]);
    expect(ungroupLosses(p, find(p, "Bass").id)).toEqual(["1 device"]);
  });

  it("maps the shortcuts", () => {
    const k = (key: string, mods: Partial<KeyboardEvent> = {}) => groupShortcut({ key, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...mods });
    expect(k("g", { metaKey: true })).toBe("group");
    expect(k("G", { ctrlKey: true, shiftKey: true })).toBe("ungroup");
    expect(k("g")).toBeNull();
    expect(k("g", { metaKey: true, altKey: true })).toBeNull();
  });
});

describe("arrangement: Cmd+G", () => {
  it("groups the selected tracks, then ungroups them (asking when something would be lost)", async () => {
    await renderApp("arrangement");
    const before = names(null);
    fireEvent.click(screen.getByRole("group", { name: "Keys track" }));
    fireEvent.click(screen.getByRole("group", { name: "Bass track" }), { shiftKey: true });
    const root = document.querySelector<HTMLElement>('[data-feature="arrangement"]')!;
    await act(async () => {
      fireEvent.keyDown(root, { key: "g", metaKey: true });
    });
    await flush();
    const group = byName("Group");
    expect(group.kind).toBe("Group");
    expect(names(group.id)).toEqual(["Keys", "Bass"]);
    expect(names(null).indexOf("Group")).toBe(before.indexOf("Keys"));
    expect(screen.getByRole("group", { name: "Group track" }).className).toContain("eth-arr-header--selected");
    // Keys and Bass show indented under the group, which folds.
    expect(screen.getByRole("group", { name: "Keys track" }).style.paddingLeft).toBe("26px");

    // The group has no devices: Cmd+Shift+G ungroups straight away.
    await act(async () => {
      fireEvent.keyDown(root, { key: "g", metaKey: true, shiftKey: true });
    });
    await flush();
    expect(Object.values(project().tracks).some((t) => t.name === "Group")).toBe(false);
    expect(names(null)).toEqual(before);
  });

  it("asks before ungrouping a group with devices", async () => {
    const m = await renderApp("arrangement");
    const g = newId();
    await act(async () => {
      await m.send(cmd("Track", { type: "GroupSelected", ids: [byName("Drums").id], group: g, name: "Drum Bus" }));
      await m.send(cmd("Device", { type: "Insert", id: newId(), track: g, device: { type: "Builtin", device: { type: "Compressor" } }, before: null } as never));
    });
    await flush();
    fireEvent.contextMenu(screen.getByRole("group", { name: "Drum Bus track" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /^Ungroup/ }));
    await flush();
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("1 device");
    expect(project().tracks[g]).toBeDefined();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Ungroup" }));
    });
    await flush();
    expect(project().tracks[g]).toBeUndefined();
    expect(byName("Drums").parent).toBeNull();
  });

  it("creates a VCA from the context menu and shows what it controls", async () => {
    await renderApp("arrangement");
    fireEvent.contextMenu(screen.getByRole("group", { name: "Keys track" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("menuitem", { name: "New VCA for Track" }));
    });
    await flush();
    const vca = Object.values(project().tracks).find((t) => t.kind === "Vca")!;
    expect(byName("Keys").vca).toBe(vca.id);
    expect(screen.getByTestId("vca-lane").textContent).toContain("Keys");
    // Another track joins from its menu.
    fireEvent.contextMenu(screen.getByRole("group", { name: "Bass track" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("menuitem", { name: `Assign to VCA “${vca.name}”` }));
    });
    await flush();
    expect(byName("Bass").vca).toBe(vca.id);
  });
});

describe("mixer: routing", () => {
  it("routes a track's output into an audio track's input, with a tap point", async () => {
    await renderApp("mixer");
    const drums = byName("Drums");
    const strip = screen.getByRole("group", { name: "Drums" });
    pickOption(within(strip).getByRole("combobox", { name: "Drums input" }), "From Keys");
    await flush();
    expect(project().tracks[drums.id]!.input).toEqual({ type: "Track", track: byName("Keys").id, tap: "PostFader" });
    expect(project().tracks[drums.id]!.monitor).toBe("In");
    pickOption(screen.getByRole("combobox", { name: "Drums input tap" }), "Pre FX");
    await flush();
    expect(project().tracks[drums.id]!.input).toMatchObject({ type: "Track", tap: "PreFx" });
  });

  it("shows VCA strips in their own section and assigns tracks to them", async () => {
    const m = await renderApp("mixer");
    const vca = newId();
    await act(async () => {
      await m.send(cmd("Track", { type: "Create", id: vca, kind: "Vca", name: "Band", color: null, parent: null, before: null }));
    });
    await flush();
    const section = screen.getByTestId("mixer-vcas");
    expect(section.querySelector('[aria-label="Band"]')).not.toBeNull();
    expect(section.querySelector('[aria-label="Band pan"]')).toBeNull();
    pickOption(screen.getByRole("combobox", { name: "Keys VCA" }), "VCA Band");
    await flush();
    expect(byName("Keys").vca).toBe(vca);
    expect(section.textContent).toContain("Keys");
  });
});
