// Missing devices: a device that couldn't be created (Live's Hybrid Reverb)
// is a placeholder that keeps its values and automation; the resolver lists it with the
// missing plugins and files, with its tracks, and replaces it (built-in or plugin) carrying
// its automation over by parameter name, in one undo step. Opening a project shows the
// dialog when anything is missing ("Resolve later" keeps everything); the top bar badge and
// the project menu reopen it.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { Device, Event } from "@/generated";
import { useContextMenuStore } from "@/kit";
import { pickOption } from "@/kit/testing";
import { useProjectStore, useSelectionStore } from "@/state";
import { TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { resetMediaRefs } from "@/features/media-refs";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { DeviceChain } from "@/features/devices";
import { ImportResolverRoot } from "./ImportResolverRoot";
import { ProjectMenu } from "@/features/project";
import { MissingPluginsStatus } from "./MissingItems";
import { missingCount } from "./missingCount";
import { autoShow, openResolver, resetResolver, setAutoShow, setResolverTransport, useResolver } from "./store";

let mock: MockTransport | undefined;

beforeAll(() => {
  const proto = HTMLElement.prototype as unknown as Record<string, unknown>;
  proto.setPointerCapture ??= () => {};
  proto.releasePointerCapture ??= () => {};
  proto.hasPointerCapture ??= () => false;
});

beforeEach(() => {
  localStorage.clear();
  resetResolver();
  resetMediaRefs();
});

afterEach(() => {
  cleanup();
  mock?.dispose();
  mock = undefined;
  setResolverTransport(null);
  useProjectStore.getState().reset();
  useSelectionStore.getState().selectTrack(null);
  usePluginStore.getState().reset();
  useContextMenuStore.setState({ menu: null });
});

const project = () => useProjectStore.getState().project!;

/** The mock engine's demo project with a missing "Live's Hybrid Reverb" (Dry/Wet automated) on its audio track. */
async function setup(ui: React.ReactNode = null): Promise<{ device: Device; t: MockTransport }> {
  const t = new MockTransport({ timers: "manual", seed: 1 });
  mock = t;
  render(
    <TransportProvider transport={t}>
      {ui}
      <ImportResolverRoot />
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  setResolverTransport(t);
  const track = Object.values(project().tracks).find((x) => x.kind === "Audio")!;
  const id = t.resolver.addMissingDevice(track.id);
  await waitFor(() => expect(project().devices[id]).toBeTruthy());
  return { device: project().devices[id]!, t };
}

const lanesOf = (device: string) => Object.values(project().automation_lanes).filter((l) => l.target.type === "DeviceParam" && l.target.device === device);

describe("missing devices in the resolver", () => {
  it("lists a missing device with its tracks and replaces it by a built-in, automation included, in one undo step", async () => {
    const { device, t } = await setup();
    const trackName = project().tracks[device.track]!.name;
    await act(() => openResolver(undefined, t));
    const dialog = await screen.findByRole("dialog", { name: "resolve missing items" });
    const row = within(dialog).getByTestId("resolver-device");
    expect(row.textContent).toContain("Live's Hybrid Reverb");
    expect(row.textContent).toContain("Ableton Live 12.2");
    expect(row.textContent).toContain(`on ${trackName}`);
    expect(row.textContent).toContain("2 saved parameters");
    // The suggestion (by name: a reverb) is preselected but not applied until switched on.
    const item = useResolver.getState().devices[0]!;
    expect(item.candidates[0]).toMatchObject({ name: "Reverb", replacement: { type: "Builtin", device: "Reverb" } });
    expect(item.apply).toBe(false);
    expect(within(dialog).getByRole("button", { name: "apply" }).hasAttribute("disabled")).toBe(true);
    fireEvent.click(within(row).getByRole("switch", { name: "apply the replacement of Live's Hybrid Reverb" }));

    const steps = () => (t as unknown as { undoStack: unknown[] }).undoStack.length;
    const before = steps();
    fireEvent.click(within(dialog).getByRole("button", { name: "apply 1 change" }));
    await waitFor(() => expect(useResolver.getState().open).toBe(false));
    const d = project().devices[device.id]!;
    expect(d.kind).toMatchObject({ type: "Builtin", device: { type: "Reverb" } });
    expect(d.name).toBe("reverb");
    // Dry/Wet 0.2 (0..1) → Mix 20 % (0..100), and the lane follows to Mix.
    expect(d.params[5]).toBeCloseTo(20);
    const lanes = lanesOf(device.id);
    expect(lanes).toHaveLength(1);
    expect(lanes[0]!.target).toEqual({ type: "DeviceParam", device: device.id, param: 5 });
    expect(steps()).toBe(before + 1);
  });

  it("choosing another device: a built-in of the same role or a plugin", async () => {
    const { t } = await setup();
    await act(() => openResolver(undefined, t));
    const dialog = await screen.findByRole("dialog", { name: "resolve missing items" });
    const row = within(dialog).getByTestId("resolver-device");
    pickOption(within(row).getByRole("combobox", { name: "choose another device for Live's Hybrid Reverb" }), "delay");
    const item = useResolver.getState().devices[0]!;
    expect(item.candidates[0]).toMatchObject({ name: "delay", reason: "chosen by you", replacement: { type: "Builtin", device: "Delay" } });
    expect(item.apply).toBe(true);
  });

  it("the device card says it is missing and resolves it from there", async () => {
    const { device } = await setup(<DeviceChain />);
    act(() => useSelectionStore.getState().selectTrack(device.track));
    const card = await screen.findByRole("region", { name: "Live's Hybrid Reverb" });
    expect(within(card).getByRole("status").textContent).toBe("missing · pass-through");
    fireEvent.click(within(card).getByRole("button", { name: "resolve Live's Hybrid Reverb" }));
    await waitFor(() => expect(useResolver.getState().focus).toBe("d:ableton:Hybrid"));
    expect(await screen.findByRole("dialog", { name: "resolve “Live's Hybrid Reverb”" })).toBeTruthy();
  });

  it("the status bar counts the missing plugins and devices and opens the dialog", async () => {
    const { t } = await setup(<MissingPluginsStatus />);
    expect(missingCount(project(), t, 0)).toBe(1);
    const badge = await screen.findByTestId("missing-items-badge");
    expect(badge.textContent).toBe("1 plugin is missing");
    fireEvent.click(badge);
    expect(await screen.findByRole("dialog", { name: "resolve missing items" })).toBeTruthy();
  });

  it("the top-left project menu lists 'Resolve missing devices and samples…' only while something is missing, and opens the dialog", async () => {
    const t = new MockTransport({ timers: "manual", seed: 1 });
    mock = t;
    render(
      <TransportProvider transport={t}>
        <ProjectMenu />
        <ImportResolverRoot />
      </TransportProvider>,
    );
    await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
    setResolverTransport(t);
    const label = "resolve missing devices and samples…";
    fireEvent.click(screen.getByRole("button", { name: "projects" }));
    await screen.findByRole("menu", { name: "project" });
    expect(screen.queryByRole("menuitem", { name: label })).toBeNull();
    fireEvent.keyDown(document.body, { key: "Escape" });

    const track = Object.values(project().tracks).find((x) => x.kind === "Audio")!;
    const id = t.resolver.addMissingDevice(track.id);
    await waitFor(() => expect(project().devices[id]).toBeTruthy());
    if (!screen.queryByRole("menu", { name: "project" })) fireEvent.click(screen.getByRole("button", { name: "projects" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: label }));
    expect(await screen.findByRole("dialog", { name: "resolve missing items" })).toBeTruthy();
  });
});

describe("on project load", () => {
  const loaded = (t: MockTransport) => {
    const e = { type: "ProjectLoaded", project: project() } as Event;
    (t as unknown as { emit(e: Event): void }).emit(e);
  };

  it("shows everything missing once, and not when turned off", async () => {
    const { t } = await setup();
    await act(async () => {
      loaded(t);
      await new Promise((r) => setTimeout(r, 0));
    });
    const dialog = await screen.findByRole("dialog", { name: "resolve missing items" });
    expect(within(dialog).getByTestId("resolver-device")).toBeTruthy();
    // "Resolve later" closes it; the item stays in the project.
    fireEvent.click(within(dialog).getByRole("button", { name: "resolve later" }));
    await waitFor(() => expect(useResolver.getState().open).toBe(false));
    expect(Object.values(project().devices).some((d) => d.kind.type === "Missing")).toBe(true);

    act(() => setAutoShow(false));
    expect(autoShow()).toBe(false);
    await act(async () => {
      loaded(t);
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(useResolver.getState().open).toBe(false);
    // Still counted (the badge, the project menu).
    await waitFor(() => expect(useResolver.getState().count).toBe(1));
  });
});
