import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ReactNode } from "react";
import type { Device } from "@/generated";
import { useProjectStore, useSelectionStore } from "@/state";
import { TransportProvider, type EngineTransport } from "@/transport";
import { isInstalled } from "./filter";
import { DeviceWindowToggle } from "@/features/devices/window";
import { PluginDeviceControls, pluginMenuEntries } from "./index";
import { AU_DELAY, DesktopFake, EQ, plugin, VERB, VST2_VERB, VST_VERB } from "./testing";
import { usePluginStore } from "./pluginStore";

let transport: EngineTransport | undefined;
afterEach(() => {
  // Unmount before resetting the stores (mounted headers would refetch the plugin list).
  cleanup();
  transport?.dispose();
  transport = undefined;
  useProjectStore.getState().reset();
  useSelectionStore.getState().selectTrack(null);
  usePluginStore.getState().reset();
});

async function renderWith<T extends EngineTransport>(t: T, ui: ReactNode): Promise<T> {
  transport = t;
  render(<TransportProvider transport={t}>{ui}</TransportProvider>);
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  return t;
}

/** A device header's plugin parts: the window toggle (in every header) + controls. */
function Header({ device }: { device: Device }) {
  return (
    <>
      <DeviceWindowToggle device={device} />
      <PluginDeviceControls device={device} />
    </>
  );
}

describe("filter helpers", () => {
  it("matches installed plugins by format and id (ids are unique per format only)", () => {
    const all = [VERB, VST_VERB, AU_DELAY, EQ, VST2_VERB];
    expect(isInstalled(all, { format: "Vst2", plugin_id: VST2_VERB.id })).toBe(true);
    expect(isInstalled(all, { format: "Vst3", plugin_id: VST2_VERB.id })).toBe(false);
    expect(isInstalled(all, { format: "Vst3", plugin_id: VST_VERB.id })).toBe(true);
    expect(isInstalled(all, { format: "Au", plugin_id: VST_VERB.id })).toBe(false);
  });
});

describe("PluginDeviceControls", () => {
  const device = (sandboxed: boolean): Device => ({
    id: "01J00000000000000000000DEV",
    track: "01J00000000000000000000TRK",
    order: "a0",
    name: "Verb",
    enabled: true,
    kind: {
      type: "Plugin",
      plugin: { format: "Clap", plugin_id: VERB.id, name: "Verb", vendor: "Acme", version: "1", sandboxed, state: null },
    },
    params: {},
    sidechain: null,
    pad: null,
  });

  it("renders nothing for built-in devices", async () => {
    await renderWith(
      new DesktopFake(),
      <div data-testid="host">
        <PluginDeviceControls device={{ ...device(false), kind: { type: "Builtin", device: { type: "Synth" } } }} />
      </div>,
    );
    expect(screen.getByTestId("host")).toBeEmptyDOMElement();
  });

  it("shows no format chip (the format doesn't matter to the user)", async () => {
    const vst2: Device = {
      ...device(false),
      kind: {
        type: "Plugin",
        plugin: { format: "Vst2", plugin_id: VST2_VERB.id, name: "Verb", vendor: "Acme", version: "1", sandboxed: false, state: null },
      },
    };
    const t = new DesktopFake();
    t.plugins = [VST2_VERB];
    await renderWith(t, <Header device={vst2} />);
    await screen.findByRole("button", { name: /window/i }).catch(() => undefined);
    expect(screen.queryByText("vst2")).toBeNull();
  });

  it("opens/closes the editor from the header's window toggle; sandboxing is a context-menu entry", async () => {
    const d = device(false);
    const t = await renderWith(new DesktopFake(), <Header device={d} />);
    const toggle = screen.getByRole("button", { name: "open Verb window" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "true"));
    expect(t.sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "OpenEditor", device: d.id } });
    // The user closed the window itself.
    t.emit({ type: "EditorClosed", device: d.id });
    await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "false"));

    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "true"));
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "false"));
    expect(t.sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "CloseEditor", device: d.id } });

    expect(screen.queryByRole("button", { name: "sandbox Verb" })).toBeNull();
    const [entry] = pluginMenuEntries(d, t);
    expect(entry).toMatchObject({ label: "run sandboxed (crash-safe, +1 block latency)" });
    (entry as { onSelect(): void }).onSelect();
    expect(pluginMenuEntries(device(true), t)).toMatchObject([{ label: "run in-process (no sandbox)" }]);
    await waitFor(() =>
      expect(t.sent.at(-1)).toEqual({
        domain: "Plugin",
        command: { type: "SetSandboxed", device: d.id, sandboxed: true },
      }),
    );
  });

  it("shows a crashed plugin as bypassed with a Reload action", async () => {
    const d = device(true);
    const t = await renderWith(new DesktopFake(), <Header device={d} />);
    t.emit({ type: "Crashed", device: d.id, message: "plugin helper exited (signal: 9)" });
    expect(screen.getByText("crashed · bypassed")).toHaveAttribute("title", "plugin helper exited (signal: 9)");
    expect(screen.getByRole("button", { name: "open Verb window" })).toBeDisabled();
    // Other devices' crashes don't matter.
    t.emit({ type: "Crashed", device: "01J0000000000000000000OTHER", message: "x" });
    fireEvent.click(screen.getByRole("button", { name: "reload Verb" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "open Verb window" })).toBeEnabled());
    expect(screen.queryByText("crashed · bypassed")).toBeNull();
    expect(t.sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "Reload", device: d.id } });
  });
});

describe("plugin formats", () => {
  const auDevice: Device = {
    id: "01J00000000000000000000AUD",
    track: "01J00000000000000000000TRK",
    order: "a0",
    name: "AUDelay",
    enabled: true,
    kind: {
      type: "Plugin",
      plugin: {
        format: "Au",
        plugin_id: AU_DELAY.id,
        name: "AUDelay",
        vendor: "Apple",
        version: "1",
        sandboxed: false,
        state: null,
      },
    },
    params: {},
    sidechain: null,
    pad: null,
  };

  /** `device` in the open project's document (the found hint is for the project's devices). */
  const inProject = (device: Device) =>
    act(() => useProjectStore.setState((s) => ({ project: { ...s.project!, devices: { ...s.project!.devices, [device.id]: device } } })));

  it("shows a plugin missing from the scanned list as bypassed; a rescan that finds it offers found · reload", async () => {
    const t = new DesktopFake();
    // The same id in another format doesn't count: lookups are by (format, id).
    t.plugins = [plugin(AU_DELAY.id, "AUDelay", "AudioEffect", "Apple", "Vst3")];
    await renderWith(t, <Header device={auDevice} />);
    inProject(auDevice);
    const missing = await screen.findByText("missing · bypassed");
    expect(missing).toHaveAttribute("title", "au plugin aufx:dely:appl is not installed. install it and rescan plugins.");
    expect(screen.getByRole("button", { name: "open AUDelay window" })).toBeDisabled();
    expect(t.sent.filter((c) => c.command.type === "List")).toHaveLength(1);

    // A rescan (from anywhere) finds it: the host still has no instance (bypassed) until it is reloaded.
    t.plugins = [AU_DELAY];
    t.emit({ type: "ScanFinished", plugins: 1, failed: [] });
    const found = (await screen.findByText("found · reload")).closest("button")!;
    expect(found).toHaveAccessibleName("reload AUDelay");
    expect(found).toHaveAttribute("title", "au plugin aufx:dely:appl was found by the last scan. load it with the state saved in the project.");
    expect(screen.queryByText("missing · bypassed")).toBeNull();
    expect(screen.getByRole("button", { name: "open AUDelay window" })).toBeDisabled();

    // One click re-instantiates it from the project's saved state; the hint goes, the window opens.
    fireEvent.click(found);
    await waitFor(() => expect(screen.getByRole("button", { name: "open AUDelay window" })).toBeEnabled());
    expect(t.sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "Reload", device: auDevice.id } });
    expect(screen.queryByText("found · reload")).toBeNull();
  });

  it("offers found · reload only for devices that were missing, and forgets it with the project", async () => {
    const t = new DesktopFake();
    t.plugins = [AU_DELAY];
    await renderWith(t, <Header device={auDevice} />);
    inProject(auDevice);
    await waitFor(() => expect(screen.getByRole("button", { name: "open AUDelay window" })).toBeEnabled());
    // Listed all along: a rescan changes nothing.
    t.emit({ type: "ScanFinished", plugins: 1, failed: [] });
    await waitFor(() => expect(t.sent.filter((c) => c.command.type === "List")).toHaveLength(2));
    expect(usePluginStore.getState().found).toEqual({});
    expect(screen.queryByText("found · reload")).toBeNull();

    // Missing, then found; then another project opens: the hint was about the old one.
    act(() => usePluginStore.getState().setPlugins([]));
    act(() => usePluginStore.getState().setPlugins([AU_DELAY]));
    expect(usePluginStore.getState().found).toEqual({ [auDevice.id]: true });
    act(() => t.events.emit({ type: "ProjectLoaded", project: useProjectStore.getState().project! }));
    expect(usePluginStore.getState().found).toEqual({});
  });

  it("reloads a missing plugin on request", async () => {
    const t = new DesktopFake();
    t.plugins = [];
    await renderWith(t, <PluginDeviceControls device={auDevice} />);
    await screen.findByText("missing · bypassed");
    fireEvent.click(screen.getByRole("button", { name: "reload AUDelay" }));
    await waitFor(() => expect(t.sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "Reload", device: auDevice.id } }));
  });

  it("crashes and reloads the same way for every format", async () => {
    const t = new DesktopFake();
    t.plugins = [AU_DELAY];
    await renderWith(t, <Header device={auDevice} />);
    await waitFor(() => expect(screen.getByRole("button", { name: "open AUDelay window" })).toBeEnabled());
    t.emit({ type: "Crashed", device: auDevice.id, message: "plugin helper exited" });
    expect(screen.getByText("crashed · bypassed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "open AUDelay window" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "reload AUDelay" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "open AUDelay window" })).toBeEnabled());
  });
});
