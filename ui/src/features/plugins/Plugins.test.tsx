import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ReactNode } from "react";
import type { Command, Device, Event, PluginDescriptor, PluginEvent, ReplyValue } from "@/generated";
import { pickOption } from "@/kit/testing";
import { useProjectStore, useSelectionStore } from "@/state";
import {
  Emitter,
  MockTransport,
  TransportProvider,
  type EngineTransport,
  type SendOptions,
  type Unsubscribe,
} from "@/transport";
import { canInsert, filterPlugins, isInstalled } from "./filter";
import { PluginBrowser, PluginDeviceControls } from "./index";
import { usePluginStore } from "./pluginStore";

const plugin = (
  id: string,
  name: string,
  category: PluginDescriptor["category"],
  vendor = "Acme",
  format: PluginDescriptor["format"] = "Clap",
): PluginDescriptor => ({
  format,
  id,
  name,
  vendor,
  version: "1.0",
  description: "",
  features: category === "Instrument" ? ["instrument", "synthesizer"] : ["audio-effect", "reverb"],
  category,
  path: `/plugins/${name}.clap`,
  sidechain_inputs: 0,
});

const VERB = plugin("com.acme.verb", "Verb", "AudioEffect");
const SYNTH = plugin("com.acme.synth", "Big Synth", "Instrument");
const EQ = plugin("org.other.eq", "Air EQ", "AudioEffect", "Other");
const VST_VERB = plugin("E7E1E4A1000000000000000000000001", "Verb", "AudioEffect", "Acme", "Vst3");
const AU_DELAY = plugin("aufx:dely:appl", "AUDelay", "AudioEffect", "Apple", "Au");

/** The desktop host as seen by the UI: MockTransport's document + host-handled plugin commands. */
class DesktopFake implements EngineTransport {
  readonly kind = "tauri" as const;
  readonly mock = new MockTransport({ timers: "manual", seed: 3 });
  readonly events = new Emitter<Event>();
  readonly sent: Command[] = [];
  plugins: PluginDescriptor[] = [VERB, SYNTH];

  connect() {
    return this.mock.connect();
  }
  async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    if (command.domain === "Plugin") {
      this.sent.push(command);
      if (command.command.type === "List") return { type: "Plugins", plugins: this.plugins };
      return { type: "Unit" };
    }
    if (command.domain === "Device" && command.command.type === "Insert" && command.command.device.type === "Plugin") {
      this.sent.push(command);
      return { type: "Unit" };
    }
    // An instrument replacing the track's instrument: one batch holding the plugin Insert.
    const isPluginInsert = (c: Command) => c.domain === "Device" && c.command.type === "Insert" && c.command.device.type === "Plugin";
    if (command.domain === "Edit" && command.command.type === "Batch" && command.command.commands.some(isPluginInsert)) {
      this.sent.push(command);
      return { type: "Unit" };
    }
    return this.mock.send(command, opts);
  }
  onEvent(listener: (event: Event) => void): Unsubscribe {
    const a = this.mock.onEvent(listener);
    const b = this.events.on(listener);
    return () => {
      a();
      b();
    };
  }
  subscribePlayhead: EngineTransport["subscribePlayhead"] = (l) => this.mock.subscribePlayhead(l);
  subscribeMeters: EngineTransport["subscribeMeters"] = (l) => this.mock.subscribeMeters(l);
  dispose() {
    this.mock.dispose();
  }
  emit(event: PluginEvent) {
    act(() => this.events.emit({ type: "Plugin", event }));
  }
}

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

const trackNamed = (name: string) => Object.values(useProjectStore.getState().project!.tracks).find((t) => t.name === name)!;
const firstDeviceOf = (name: string) =>
  Object.values(useProjectStore.getState().project!.devices)
    .filter((d) => d.track === trackNamed(name).id)
    .sort((a, b) => (a.order < b.order ? -1 : 1))[0]!;

describe("filter helpers", () => {
  it("searches name, vendor, category and features; sorts by name", () => {
    const all = [VERB, SYNTH, EQ];
    expect(filterPlugins(all, "").map((p) => p.name)).toEqual(["Air EQ", "Big Synth", "Verb"]);
    expect(filterPlugins(all, "acme").map((p) => p.name)).toEqual(["Big Synth", "Verb"]);
    expect(filterPlugins(all, "REVERB acme").map((p) => p.name)).toEqual(["Verb"]);
    expect(filterPlugins(all, "instrument").map((p) => p.name)).toEqual(["Big Synth"]);
    expect(filterPlugins(all, "nothing")).toEqual([]);
  });

  it("filters by format and searches format names", () => {
    const all = [VERB, VST_VERB, AU_DELAY, EQ];
    expect(filterPlugins(all, "").map((p) => `${p.name}/${p.format}`)).toEqual([
      "Air EQ/Clap",
      "AUDelay/Au",
      "Verb/Clap",
      "Verb/Vst3",
    ]);
    expect(filterPlugins(all, "", "Vst3")).toEqual([VST_VERB]);
    expect(filterPlugins(all, "", "Au")).toEqual([AU_DELAY]);
    expect(filterPlugins(all, "verb", "Clap")).toEqual([EQ, VERB]);
    expect(filterPlugins(all, "vst3")).toEqual([VST_VERB]);
    expect(filterPlugins(all, "au apple")).toEqual([AU_DELAY]);
    // Ids are unique per format only.
    expect(isInstalled(all, { format: "Vst3", plugin_id: VST_VERB.id })).toBe(true);
    expect(isInstalled(all, { format: "Au", plugin_id: VST_VERB.id })).toBe(false);
  });

  it("only allows instruments on MIDI tracks", () => {
    const midi = { kind: "Midi" } as Parameters<typeof canInsert>[1];
    const audio = { kind: "Audio" } as Parameters<typeof canInsert>[1];
    expect(canInsert(SYNTH, midi)).toBe(true);
    expect(canInsert(SYNTH, audio)).toBe(false);
    expect(canInsert(VERB, audio)).toBe(true);
    expect(canInsert(VERB, undefined)).toBe(false);
  });
});

describe("PluginBrowser", () => {
  it("tells web users plugins are desktop-only", async () => {
    await renderWith(new MockTransport({ timers: "manual", seed: 3 }), <PluginBrowser />);
    expect(screen.getByText("Plugins are available in the desktop app.")).toBeInTheDocument();
  });

  it("lists, searches and inserts plugins on the selected track", async () => {
    const t = await renderWith(new DesktopFake(), <PluginBrowser />);
    const list = await screen.findByRole("list", { name: "Plugins" });
    await within(list).findByText("Verb");
    expect(within(list).getAllByRole("button").map((b) => b.textContent)).toEqual([
      "Big SynthAcmeInstrumentCLAP",
      "VerbAcmeEffectCLAP",
    ]);
    expect(screen.getByText("Insert on: Keys")).toBeInTheDocument();

    // Instrument on a MIDI track: it replaces the track's instrument (the Synth), in place, one step.
    fireEvent.click(within(list).getByText("Big Synth"));
    await screen.findByText("Inserted Big Synth on Keys");
    const synth = firstDeviceOf("Keys").id;
    expect(t.sent.at(-1)).toMatchObject({
      domain: "Edit",
      command: {
        type: "Batch",
        label: "Replace Instrument",
        commands: [
          {
            domain: "Device",
            command: {
              type: "Insert",
              track: trackNamed("Keys").id,
              device: { type: "Plugin", plugin_id: SYNTH.id, format: "Clap", sandboxed: null },
              before: synth,
            },
          },
          { domain: "Device", command: { type: "Remove", id: synth } },
        ],
      },
    });

    // Effects go at the end; instruments can't go on audio tracks.
    act(() => useSelectionStore.getState().selectTrack(trackNamed("Drums").id));
    expect(screen.getByText("Insert on: Drums")).toBeInTheDocument();
    expect(within(list).getByText("Big Synth").closest("button")).toBeDisabled();
    fireEvent.click(within(list).getByText("Verb"));
    await screen.findByText("Inserted Verb on Drums");
    expect(t.sent.at(-1)).toMatchObject({
      command: { type: "Insert", track: trackNamed("Drums").id, before: null },
    });

    fireEvent.change(screen.getByRole("searchbox", { name: "Search plugins" }), { target: { value: "synth" } });
    expect(within(list).getAllByRole("button").map((b) => b.textContent)).toEqual(["Big SynthAcmeInstrumentCLAP"]);
    fireEvent.change(screen.getByRole("searchbox", { name: "Search plugins" }), { target: { value: "zzz" } });
    expect(within(list).getByText("No match.")).toBeInTheDocument();
  });

  it("rescans with progress and refreshes the list", async () => {
    const t = await renderWith(new DesktopFake(), <PluginBrowser />);
    await screen.findByText("Verb");
    fireEvent.click(screen.getByRole("button", { name: "Rescan" }));
    expect(t.sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "Rescan" } });
    t.emit({ type: "ScanProgress", done: 1, total: 3, current: "/plugins/Air EQ.clap" });
    expect(screen.getByText("Scanning 1/3")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Scanning…" })).toBeDisabled();
    t.plugins = [VERB, SYNTH, EQ];
    t.emit({ type: "ScanFinished", plugins: 3, failed: [{ path: "/plugins/bad.clap", message: "crashed" }] });
    await screen.findByText("Air EQ");
    expect(screen.getByText("1 plugin failed to load")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Rescan" })).toBeEnabled();
    // A scan-level failure (no path) shows its reason.
    t.emit({ type: "ScanFinished", plugins: 0, failed: [{ path: "", message: "plugin scanner binary not found" }] });
    expect(await screen.findByText("Plugin scan failed: plugin scanner binary not found")).toBeInTheDocument();
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

  it("opens/closes the editor and toggles the sandbox", async () => {
    const d = device(false);
    const t = await renderWith(new DesktopFake(), <PluginDeviceControls device={d} />);
    fireEvent.click(screen.getByRole("button", { name: "Open Verb editor" }));
    await screen.findByRole("button", { name: "Close Verb editor" });
    expect(t.sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "OpenEditor", device: d.id } });
    // The user closed the window itself.
    t.emit({ type: "EditorClosed", device: d.id });
    expect(screen.getByRole("button", { name: "Open Verb editor" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Open Verb editor" }));
    fireEvent.click(await screen.findByRole("button", { name: "Close Verb editor" }));
    await screen.findByRole("button", { name: "Open Verb editor" });
    expect(t.sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "CloseEditor", device: d.id } });

    const sandbox = screen.getByRole("button", { name: "Sandbox Verb" });
    expect(sandbox).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(sandbox);
    await waitFor(() =>
      expect(t.sent.at(-1)).toEqual({
        domain: "Plugin",
        command: { type: "SetSandboxed", device: d.id, sandboxed: true },
      }),
    );
  });

  it("shows a crashed plugin as bypassed with a Reload action", async () => {
    const d = device(true);
    const t = await renderWith(new DesktopFake(), <PluginDeviceControls device={d} />);
    expect(screen.getByRole("button", { name: "Sandbox Verb" })).toHaveAttribute("aria-pressed", "true");
    t.emit({ type: "Crashed", device: d.id, message: "plugin helper exited (signal: 9)" });
    expect(screen.getByText("crashed · bypassed")).toHaveAttribute("title", "plugin helper exited (signal: 9)");
    expect(screen.queryByRole("button", { name: "Open Verb editor" })).toBeNull();
    // Other devices' crashes don't matter.
    t.emit({ type: "Crashed", device: "01J0000000000000000000OTHER", message: "x" });
    fireEvent.click(screen.getByRole("button", { name: "Reload Verb" }));
    await screen.findByRole("button", { name: "Open Verb editor" });
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

  it("badges every plugin with its format, filters by format and inserts with the format", async () => {
    const t = new DesktopFake();
    t.plugins = [VERB, VST_VERB, AU_DELAY, SYNTH];
    await renderWith(t, <PluginBrowser />);
    const list = await screen.findByRole("list", { name: "Plugins" });
    await within(list).findByText("AUDelay");
    expect(within(list).getAllByRole("button").map((b) => b.textContent)).toEqual([
      "AUDelayAppleEffectAU",
      "Big SynthAcmeInstrumentCLAP",
      "VerbAcmeEffectCLAP",
      "VerbAcmeEffectVST3",
    ]);

    const format = screen.getByRole("combobox", { name: "Plugin format" });
    pickOption(format, { value: "Vst3" });
    expect(within(list).getAllByRole("button").map((b) => b.textContent)).toEqual(["VerbAcmeEffectVST3"]);
    act(() => useSelectionStore.getState().selectTrack(trackNamed("Drums").id));
    fireEvent.click(within(list).getByText("Verb"));
    await screen.findByText("Inserted Verb on Drums");
    expect(t.sent.at(-1)).toMatchObject({
      command: {
        type: "Insert",
        device: { type: "Plugin", plugin_id: VST_VERB.id, format: "Vst3", sandboxed: null },
      },
    });

    pickOption(format, { value: "Au" });
    fireEvent.click(within(list).getByText("AUDelay"));
    await screen.findByText("Inserted AUDelay on Drums");
    expect(t.sent.at(-1)).toMatchObject({
      command: { type: "Insert", device: { type: "Plugin", plugin_id: AU_DELAY.id, format: "Au" } },
    });

    pickOption(format, { value: "All" });
    expect(within(list).getAllByRole("button")).toHaveLength(4);
  });

  it("shows a plugin missing from the scanned list as bypassed, until a rescan finds it", async () => {
    const t = new DesktopFake();
    // The same id in another format doesn't count: lookups are by (format, id).
    t.plugins = [plugin(AU_DELAY.id, "AUDelay", "AudioEffect", "Apple", "Vst3")];
    await renderWith(t, <PluginDeviceControls device={auDevice} />);
    const missing = await screen.findByText("missing · bypassed");
    expect(missing).toHaveAttribute(
      "title",
      "AU plugin aufx:dely:appl is not installed. Rescan plugins, then reload.",
    );
    expect(screen.queryByRole("button", { name: "Open AUDelay editor" })).toBeNull();
    expect(t.sent.filter((c) => c.command.type === "List")).toHaveLength(1);

    // A rescan (from anywhere) finds it.
    t.plugins = [AU_DELAY];
    t.emit({ type: "ScanFinished", plugins: 1, failed: [] });
    await screen.findByRole("button", { name: "Open AUDelay editor" });
    expect(screen.queryByText("missing · bypassed")).toBeNull();
  });

  it("reloads a missing plugin on request", async () => {
    const t = new DesktopFake();
    t.plugins = [];
    await renderWith(t, <PluginDeviceControls device={auDevice} />);
    await screen.findByText("missing · bypassed");
    fireEvent.click(screen.getByRole("button", { name: "Reload AUDelay" }));
    await waitFor(() =>
      expect(t.sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "Reload", device: auDevice.id } }),
    );
  });

  it("crashes and reloads the same way for every format", async () => {
    const t = new DesktopFake();
    t.plugins = [AU_DELAY];
    await renderWith(t, <PluginDeviceControls device={auDevice} />);
    await screen.findByRole("button", { name: "Open AUDelay editor" });
    t.emit({ type: "Crashed", device: auDevice.id, message: "plugin helper exited" });
    expect(screen.getByText("crashed · bypassed")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reload AUDelay" }));
    await screen.findByRole("button", { name: "Open AUDelay editor" });
  });
});
