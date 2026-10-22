import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, Device, ReplyValue, SandboxDefault } from "@/generated";
import { useProjectStore } from "@/state";
import { TransportProvider, type EngineTransport, type SendOptions } from "@/transport";
import { PluginDeviceControls, pluginMenuEntries } from "./index";
import { usePluginStore } from "./pluginStore";
import { SandboxDefaultsSettings } from "./SandboxDefaultsSettings";
import { isSandboxDefault, useSandboxDefaults } from "./sandboxDefaults";
import { DesktopFake, SYNTH, VERB } from "./testing";

/** The desktop host, with its "sandboxed by default" list. */
class SandboxFake extends DesktopFake {
  list: SandboxDefault[] = [{ format: "Clap", plugin_id: VERB.id, name: "Verb" }];
  override async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    if (command.domain === "Plugin") {
      const c = command.command;
      if (c.type === "ListSandboxDefaults") {
        this.sent.push(command);
        return { type: "SandboxDefaults", plugins: structuredClone(this.list) };
      }
      if (c.type === "SetSandboxDefault") {
        this.sent.push(command);
        const same = (d: SandboxDefault) => d.format === c.format && d.plugin_id === c.plugin_id;
        this.list = this.list.filter((d) => !same(d));
        if (c.sandboxed) this.list.push({ format: c.format, plugin_id: c.plugin_id, name: c.name ?? c.plugin_id });
        return { type: "SandboxDefaults", plugins: structuredClone(this.list) };
      }
    }
    return super.send(command, opts);
  }
}

let transport: EngineTransport | undefined;
afterEach(() => {
  cleanup();
  transport?.dispose();
  transport = undefined;
  useProjectStore.getState().reset();
  usePluginStore.getState().reset();
  useSandboxDefaults.getState().reset();
});

async function renderWith<T extends EngineTransport>(t: T, ui: React.ReactNode): Promise<T> {
  transport = t;
  render(<TransportProvider transport={t}>{ui}</TransportProvider>);
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  return t;
}

const device = (sandboxed: boolean, plugin = VERB): Device => ({
  id: "01J00000000000000000000DEV",
  track: "01J00000000000000000000TRK",
  order: "a0",
  name: plugin.name,
  enabled: true,
  kind: {
    type: "Plugin",
    plugin: { format: plugin.format, plugin_id: plugin.id, name: plugin.name, vendor: "Acme", version: "1", sandboxed, state: null },
  },
  params: {},
  sidechain: null,
  pad: null,
});

describe("isSandboxDefault", () => {
  it("matches by format and plugin id, ids case-insensitively", () => {
    const list: SandboxDefault[] = [{ format: "Vst3", plugin_id: "ABCDEF", name: "X" }];
    expect(isSandboxDefault(list, { format: "Vst3", plugin_id: "abcdef" })).toBe(true);
    expect(isSandboxDefault(list, { format: "Clap", plugin_id: "ABCDEF" })).toBe(false);
    expect(isSandboxDefault(null, { format: "Vst3", plugin_id: "ABCDEF" })).toBe(false);
  });
});

describe("Settings > Plugins > Sandboxing", () => {
  it("lists the engine host's plugins, removes one and adds one from the plugin picker", async () => {
    const host = new SandboxFake();
    await renderWith(host, <SandboxDefaultsSettings transport={host} />);
    act(() => usePluginStore.getState().setPlugins([VERB, SYNTH]));
    const list = await screen.findByRole("list", { name: "sandboxed by default" });
    expect(within(list).getByText("Verb")).toBeInTheDocument();
    expect(within(list).getByText("clap")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "stop sandboxing Verb by default" }));
    await screen.findByText("no plugins yet: new plugins run in-process.");
    expect(host.list).toEqual([]);

    fireEvent.click(screen.getByRole("button", { name: "add a plugin…" }));
    fireEvent.click((await screen.findByText("Big Synth")).closest<HTMLElement>("[data-key]")!);
    await waitFor(() => expect(host.list).toEqual([{ format: "Clap", plugin_id: SYNTH.id, name: "Big Synth" }]));
    expect(within(await screen.findByRole("list", { name: "sandboxed by default" })).getByText("Big Synth")).toBeInTheDocument();
  });

  it("says so when the engine host has no sandbox (the web)", async () => {
    const host = new DesktopFake();
    await renderWith(host, <SandboxDefaultsSettings transport={host} />);
    expect(await screen.findByText("sandboxed plugins need the desktop app's engine.")).toBeInTheDocument();
  });
});

describe("the device menu's always sandbox", () => {
  it("adds the plugin to the list and sandboxes this instance; listed, it only takes it off the list", async () => {
    const host = new SandboxFake();
    host.list = [];
    const d = device(false);
    await renderWith(host, <PluginDeviceControls device={d} />);
    await waitFor(() => expect(useSandboxDefaults.getState().list).toEqual([]));

    const entries = pluginMenuEntries(d, host);
    expect(entries.map((e) => (e === "separator" ? e : e.label))).toEqual(["run sandboxed (crash-safe, +1 block latency)", "always sandbox this plugin"]);
    (entries[1] as { onSelect(): void }).onSelect();
    await waitFor(() => expect(host.sent.at(-1)).toEqual({ domain: "Plugin", command: { type: "SetSandboxed", device: d.id, sandboxed: true } }));
    expect(host.list).toEqual([{ format: "Clap", plugin_id: VERB.id, name: "Verb" }]);

    // Listed now: the entry removes it, and leaves this (sandboxed) instance as it is.
    const sent = host.sent.length;
    const [, stop] = pluginMenuEntries(device(true), host);
    expect(stop).toMatchObject({ label: "stop sandboxing this plugin by default" });
    (stop as { onSelect(): void }).onSelect();
    await waitFor(() => expect(host.list).toEqual([]));
    expect(host.sent.slice(sent).map((c) => c.command.type)).toEqual(["SetSandboxDefault"]);
  });

  it("is not offered while the engine host has no such setting", async () => {
    const host = new DesktopFake();
    const d = device(false);
    await renderWith(host, <PluginDeviceControls device={d} />);
    await waitFor(() => expect(useSandboxDefaults.getState().unsupported).toBe(true));
    expect(pluginMenuEntries(d, host)).toHaveLength(1);
  });
});
