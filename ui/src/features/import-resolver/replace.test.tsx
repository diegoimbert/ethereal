// "Replace device…" (owner requests): any device but a rack, factory or plugin, can be
// swapped for a factory device or a plugin of the same role, with "Retain automations and
// parameters" (on by default) or without, in one undo step.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Device, DeviceId } from "@/generated";
import { useContextMenuStore, type ContextMenuItem } from "@/kit";
import { pickDevice } from "@/features/devices/testing";
import { useProjectStore, useSelectionStore } from "@/state";
import { cmd, MOCK_MEGA_PLUGIN_ID, MockTransport, newId, TransportProvider, type EngineTransport } from "@/transport";
import { MOCK_MINI_PLUGIN_ID } from "@/transport/mock/plugins";
import { DeviceChain } from "@/features/devices";
import { replaceDeviceEntries, replaceDeviceLabel } from "./menus";
import { ReplaceDialog } from "./ReplaceDialog";
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
      <ReplaceDialog transport={t} />
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

const pluginId = (d: Device | undefined) => (d?.kind.type === "Plugin" ? d.kind.plugin.plugin_id : null);

describe("Replace device…", () => {
  it("chooses the replacement in the shared plugin picker: searchable, the current plugin left out", async () => {
    await setup();
    const dialog = await openFromHeader();
    const trigger = within(dialog).getByRole("button", { name: "replacement device" });
    fireEvent.click(trigger);
    const popover = await screen.findByRole("dialog", { name: "replacement device" });
    expect(within(popover).getByRole("searchbox", { name: "search effects" })).toBeInTheDocument();
    const list = within(await within(popover).findByRole("list", { name: "effects" }));
    await waitFor(() => expect(list.getByRole("button", { name: "Mock Mini" })).toBeInTheDocument());
    expect(list.queryByRole("button", { name: "Mock Mega" })).toBeNull();
    fireEvent.change(within(popover).getByRole("searchbox"), { target: { value: "zzz" } });
    expect(list.getByText("no match.")).toBeInTheDocument();
  });

  it("retains values and automation by parameter name, reports what can't be kept, one undo step", async () => {
    const { t, device, cutoffLane, releaseLane } = await setup();
    const before = structuredClone(project());
    const dialog = await openFromHeader();
    expect(within(dialog).getByRole("switch", { name: "retain automations and parameters" })).toHaveAttribute("aria-checked", "true");
    // The current plugin is not offered.
    await pickDevice(within(dialog).getByRole("button", { name: "replacement device" }), "Mock Mini");
    const summary = await within(dialog).findByTestId("replace-summary");
    expect(summary).toHaveTextContent("parameters matched by name: 1 parameter kept.");
    expect(summary).toHaveTextContent("can’t keep: Release 5");
    expect(summary).toHaveTextContent("1 automation lane of unmatched parameters will be removed.");
    // The preview changed nothing.
    expect(pluginId(project().devices[device])).toBe(MOCK_MEGA_PLUGIN_ID);

    fireEvent.click(within(dialog).getByRole("button", { name: "replace" }));
    await waitFor(() => expect(pluginId(project().devices[device])).toBe(MOCK_MINI_PLUGIN_ID));
    const d = project().devices[device]!;
    expect(d.name).toBe("Mock Mini");
    expect(d.order).toBe(before.devices[device]!.order);
    expect(d.params).toEqual({ 101: 0.4 });
    expect(project().automation_lanes[cutoffLane]?.target).toEqual({ type: "DeviceParam", device, param: 101 });
    expect(project().automation_lanes[releaseLane]).toBeUndefined();
    expect(useReplace.getState().open).toBe(false);

    await act(() => t.send(cmd("Edit", { type: "Undo" })));
    await waitFor(() => expect(pluginId(project().devices[device])).toBe(MOCK_MEGA_PLUGIN_ID));
    expect(project().devices[device]).toEqual(before.devices[device]);
    expect(project().automation_lanes).toEqual(before.automation_lanes);
  });

  it("without retain: defaults, and the parameter lanes are removed", async () => {
    const { device, cutoffLane, releaseLane } = await setup();
    const dialog = await openFromHeader();
    await pickDevice(within(dialog).getByRole("button", { name: "replacement device" }), "Mock Mini");
    await within(dialog).findByTestId("replace-summary");
    fireEvent.click(within(dialog).getByRole("switch", { name: "retain automations and parameters" }));
    await waitFor(() =>
      expect(within(dialog).getByTestId("replace-summary")).toHaveTextContent(
        "the new device starts at its defaults. 2 automation lanes and the parameter mappings are removed",
      ),
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "replace" }));
    await waitFor(() => expect(pluginId(project().devices[device])).toBe(MOCK_MINI_PLUGIN_ID));
    expect(project().devices[device]!.params).toEqual({});
    expect(project().automation_lanes[cutoffLane]).toBeUndefined();
    expect(project().automation_lanes[releaseLane]).toBeUndefined();
  });

  it("lists factory devices and plugins of the same role, never a rack nor the device itself", async () => {
    await setup();
    const dialog = await openFromHeader();
    fireEvent.click(within(dialog).getByRole("button", { name: "replacement device" }));
    const popover = await screen.findByRole("dialog", { name: "replacement device" });
    const list = within(await within(popover).findByRole("list", { name: "effects" }));
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
    const dialog = await openFromHeader(before.name);
    await pickDevice(within(dialog).getByRole("button", { name: "replacement device" }), /^gate$/i);
    const summary = await within(dialog).findByTestId("replace-summary");
    expect(summary).toHaveTextContent("parameters matched by name");
    fireEvent.click(within(dialog).getByRole("button", { name: "replace" }));
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
