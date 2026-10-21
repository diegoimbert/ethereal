// "Replace device…" (owner requests): any device but a rack, factory or plugin, can be
// swapped for a factory device or a plugin of the same role, with "Retain automations and
// parameters" (on by default) or without, in one undo step: a popover under the device's
// header, the device library and the retain toggle; a row replaces at once.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Device, DeviceId } from "@/generated";
import { useContextMenuStore, type ContextMenuItem } from "@/kit";
import { useNotices } from "@/features/notifications";
import { useProjectStore, useSelectionStore } from "@/state";
import { cmd, newId, TransportProvider, type EngineTransport } from "@/transport";
import { MOCK_MEGA_PLUGIN_ID, MockTransport } from "@/transport/mock";
import { MOCK_MINI_PLUGIN_ID } from "@/transport/mock/plugins";
import { DeviceChain } from "@/features/devices";
import { replaceDeviceEntries, replaceDeviceLabel } from "./menus";
import { resetReplace, setReplaceTransport, useReplace } from "./replace";

let mock: MockTransport | undefined;

beforeAll(() => {
  const proto = HTMLElement.prototype as unknown as Record<string, unknown>;
  proto.setPointerCapture ??= () => {};
  proto.releasePointerCapture ??= () => {};
  proto.hasPointerCapture ??= () => false;
});

afterEach(() => {
  cleanup();
  mock?.dispose();
  mock = undefined;
  setReplaceTransport(null);
  resetReplace();
  useProjectStore.getState().reset();
  useSelectionStore.getState().selectTrack(null);
  useContextMenuStore.setState({ menu: null });
  useNotices.getState().clear();
});

const project = () => useProjectStore.getState().project!;
const menuItem = (label: string) =>
  (useContextMenuStore.getState().menu?.items ?? []).find((e): e is ContextMenuItem => e !== "separator" && e.label === label);

/**
 * The mock engine with a Mock Mega device: "Cutoff 1" at 0.4 with a lane, and a lane on
 * "Release 5" (Mock Mini has "Cutoff 1" at id 101, and no Release).
 */
async function setup(): Promise<{ t: MockTransport; device: DeviceId; cutoffLane: string; releaseLane: string }> {
  const t = new MockTransport({ timers: "manual", seed: 1 });
  mock = t;
  render(
    <TransportProvider transport={t}>
      <DeviceChain />
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  setReplaceTransport(t);
  const track = Object.values(project().tracks).find((x) => x.kind === "Audio")!;
  const device = newId() as DeviceId;
  await t.send(
    cmd("Device", {
      type: "Insert",
      id: device,
      track: track.id,
      device: { type: "Plugin", plugin_id: MOCK_MEGA_PLUGIN_ID, format: "Clap", sandboxed: null },
      before: null,
    }),
  );
  await t.send(cmd("Device", { type: "SetParam", device, param: 1, value: 0.4 }));
  const lane = async (param: number) => {
    const id = newId();
    await t.send(cmd("Automation", { type: "CreateLane", id, owner: { type: "Track", track: track.id }, target: { type: "DeviceParam", device, param } }));
    return id;
  };
  const cutoffLane = await lane(1);
  const releaseLane = await lane(5);
  await waitFor(() => expect(Object.keys(project().automation_lanes)).toContain(releaseLane));
  act(() => useSelectionStore.getState().selectTrack(track.id));
  return { t, device, cutoffLane, releaseLane };
}

async function openFromHeader(name = "Mock Mega"): Promise<HTMLElement> {
  const card = await screen.findByRole("region", { name });
  fireEvent.contextMenu(card.querySelector(".eth-device__header")!);
  const item = menuItem(replaceDeviceLabel());
  expect(item).toBeTruthy();
  await act(async () => item!.onSelect());
  return screen.findByRole("dialog", { name: `replace “${name}”` });
}

/** The popover's list of `kind` (effects, instruments…), loaded. */
async function listIn(popover: HTMLElement, kind = "effects") {
  const list = within(await within(popover).findByRole("list", { name: kind }));
  await waitFor(() => expect(list.queryAllByRole("button").length).toBeGreaterThan(0));
  return list;
}

const pluginId = (d: Device | undefined) => (d?.kind.type === "Plugin" ? d.kind.plugin.plugin_id : null);

describe("Replace device…", () => {
  it("is a popover with the device library (searchable, the current plugin left out) and the retain toggle", async () => {
    await setup();
    const popover = await openFromHeader();
    expect(within(popover).getByRole("searchbox", { name: "search effects" })).toBeInTheDocument();
    const list = await listIn(popover);
    await waitFor(() => expect(list.getByRole("button", { name: "Mock Mini" })).toBeInTheDocument());
    expect(list.queryByRole("button", { name: "Mock Mega" })).toBeNull();
    expect(within(popover).getByRole("switch", { name: "retain automations and parameters" })).toHaveAttribute("aria-checked", "true");
    // What it keeps: the toggle's tooltip (its title), not text under it.
    expect(within(popover).getByRole("switch").closest("[title]")).toHaveAttribute("title", expect.stringMatching(/^matching parameters keep their values/));
    fireEvent.change(within(popover).getByRole("searchbox"), { target: { value: "zzz" } });
    expect(list.getByText("no match.")).toBeInTheDocument();
  });

  it("a row replaces at once, retaining values and automation by name; says what it couldn't keep; one undo step", async () => {
    const { t, device, cutoffLane, releaseLane } = await setup();
    const before = structuredClone(project());
    const popover = await openFromHeader();
    const list = await listIn(popover);
    fireEvent.click(await list.findByRole("button", { name: "Mock Mini" }));
    await waitFor(() => expect(pluginId(project().devices[device])).toBe(MOCK_MINI_PLUGIN_ID));
    const d = project().devices[device]!;
    expect(d.name).toBe("Mock Mini");
    expect(d.order).toBe(before.devices[device]!.order);
    expect(d.params).toEqual({ 101: 0.4 });
    expect(project().automation_lanes[cutoffLane]?.target).toEqual({ type: "DeviceParam", device, param: 101 });
    expect(project().automation_lanes[releaseLane]).toBeUndefined();
    expect(useReplace.getState().open).toBe(false);
    expect(useNotices.getState().notices.at(-1)?.message).toBe(
      "replaced with Mock Mini. can’t keep: Release 5. 1 automation lane of unmatched parameters removed (undo brings them back).",
    );

    await act(() => t.send(cmd("Edit", { type: "Undo" })));
    await waitFor(() => expect(pluginId(project().devices[device])).toBe(MOCK_MEGA_PLUGIN_ID));
    expect(project().devices[device]).toEqual(before.devices[device]);
    expect(project().automation_lanes).toEqual(before.automation_lanes);
  });

  it("without retain: defaults, and the parameter lanes are removed", async () => {
    const { device, cutoffLane, releaseLane } = await setup();
    const popover = await openFromHeader();
    fireEvent.click(within(popover).getByRole("switch", { name: "retain automations and parameters" }));
    expect(within(popover).getByRole("switch").closest("[title]")).toHaveAttribute("title", expect.stringMatching(/^the new device starts at its defaults/));
    const list = await listIn(popover);
    fireEvent.click(await list.findByRole("button", { name: "Mock Mini" }));
    await waitFor(() => expect(pluginId(project().devices[device])).toBe(MOCK_MINI_PLUGIN_ID));
    expect(project().devices[device]!.params).toEqual({});
    expect(project().automation_lanes[cutoffLane]).toBeUndefined();
    expect(project().automation_lanes[releaseLane]).toBeUndefined();
  });

  it("lists factory devices and plugins of the same role, never a rack nor the device itself", async () => {
    await setup();
    const popover = await openFromHeader();
    const list = await listIn(popover);
    await waitFor(() => expect(list.getByRole("button", { name: /^reverb$/i })).toBeInTheDocument());
    expect(list.getByRole("button", { name: "Mock Mini" })).toBeInTheDocument();
    expect(list.queryByRole("button", { name: "Mock Mega" })).toBeNull();
    expect(list.queryByRole("button", { name: /audio effect rack/i })).toBeNull();
    // Audio effects only: no instrument, no MIDI effect.
    expect(list.queryByRole("button", { name: /^synth$/i })).toBeNull();
    expect(list.queryByRole("button", { name: /^arpeggiator$/i })).toBeNull();
  });

  it("swaps a factory device for another one, one undo step", async () => {
    const { t } = await setup();
    const track = Object.values(project().tracks).find((x) => x.kind === "Audio")!;
    const comp = newId() as DeviceId;
    await t.send(cmd("Device", { type: "Insert", id: comp, track: track.id, device: { type: "Builtin", device: { type: "Compressor" } }, before: null }));
    await waitFor(() => expect(project().devices[comp]).toBeDefined());
    const before = structuredClone(project().devices[comp]!);
    const popover = await openFromHeader(before.name);
    const list = await listIn(popover);
    fireEvent.click(await list.findByRole("button", { name: /^gate$/i }));
    await waitFor(() => expect(project().devices[comp]?.kind).toEqual({ type: "Builtin", device: { type: "Gate" } }));
    const d = project().devices[comp]!;
    expect(d.name).toBe("gate");
    expect(d.order).toBe(before.order);
    await act(() => t.send(cmd("Edit", { type: "Undo" })));
    await waitFor(() => expect(project().devices[comp]).toEqual(before));
  });

  it("is offered for every device but a rack, on every host", async () => {
    const { device } = await setup();
    const plugin = project().devices[device]!;
    const labels = (d: Device, t: EngineTransport | undefined) => replaceDeviceEntries(d, t).map((e) => (e === "separator" ? "—" : e.label));
    expect(labels(plugin, mock)).toEqual([replaceDeviceLabel()]);
    const builtin = Object.values(project().devices).find((d) => d.kind.type === "Builtin")!;
    expect(labels(builtin, mock)).toEqual([replaceDeviceLabel()]);
    const web = { kind: "wasm" } as unknown as EngineTransport;
    expect(labels(builtin, web)).toEqual([replaceDeviceLabel()]);
    const rack: Device = { ...builtin, kind: { type: "Builtin", device: { type: "AudioEffectRack" } } };
    expect(labels(rack, mock)).toEqual([]);
    const drums: Device = { ...builtin, kind: { type: "Builtin", device: { type: "DrumRack" } } };
    expect(labels(drums, mock)).toEqual([]);
  });
});
