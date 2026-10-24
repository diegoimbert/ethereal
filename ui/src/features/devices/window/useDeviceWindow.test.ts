import { afterEach, describe, expect, it } from "vitest";
import type { Command, Device } from "@/generated";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { flush } from "@/features/devices/testUtils";
import type { EngineTransport } from "@/transport";
import { resetDeviceWindows, useDeviceWindows } from "./store";
import { setDeviceWindow, showDeviceWindow } from "./useDeviceWindow";
import { useNotices } from "@/features/notifications";
import { CommandFailedError } from "@/transport";

/** A transport that records what it is sent (plugin commands succeed). */
function recorder(): EngineTransport & { sent: Command[] } {
  const sent: Command[] = [];
  return { sent, send: (c: Command) => (sent.push(c), Promise.resolve({ type: "Unit" })) } as unknown as EngineTransport & { sent: Command[] };
}
const device = (id: string, plugin: boolean) =>
  ({ id, kind: plugin ? { type: "Plugin", plugin: { id: "p" } } : { type: "Builtin", device: { type: "Eq" } } }) as unknown as Device;

afterEach(() => {
  resetDeviceWindows();
  usePluginStore.setState({ editors: {}, crashed: {}, opening: {}, loading: [] });
  useNotices.getState().clear();
});

const types = (t: { sent: Command[] }) => t.sent.map((c) => (c as { command: { type: string } }).command.type);
const notices = () => useNotices.getState().notices.map((n) => `${n.level}: ${n.message}`);

describe("showDeviceWindow (a click on a device card)", () => {
  it("never opens a chain rack's window (its macros and chains are on its card)", () => {
    const t = recorder();
    const rack = { id: "r", kind: { type: "Builtin", device: { type: "InstrumentRack" } } } as unknown as Device;
    showDeviceWindow(t, rack);
    expect(useDeviceWindows.getState().open).toEqual([]);
  });

  it("opens a factory device's window, and raises it when open", () => {
    const t = recorder();
    showDeviceWindow(t, device("a", false));
    showDeviceWindow(t, device("b", false));
    expect(useDeviceWindows.getState().open).toEqual(["a", "b"]);
    showDeviceWindow(t, device("a", false));
    expect(useDeviceWindows.getState().open).toEqual(["b", "a"]);
    expect(t.sent).toEqual([]);
  });

  it("opens a plugin's editor, and shows it again (to the front) when open, never closes it", async () => {
    const t = recorder();
    showDeviceWindow(t, device("p1", true));
    await flush();
    expect(usePluginStore.getState().editors["p1"]).toBeTruthy();
    showDeviceWindow(t, device("p1", true));
    await flush();
    expect(t.sent.map((c) => (c as { command: { type: string } }).command.type)).toEqual(["OpenEditor", "OpenEditor"]);
    expect(usePluginStore.getState().editors["p1"]).toBeTruthy();
  });

  it("doesn't ask a crashed plugin, and says why", () => {
    const t = recorder();
    usePluginStore.setState({ crashed: { p2: "boom" } });
    showDeviceWindow(t, { ...device("p2", true), name: "Verb" });
    expect(t.sent).toEqual([]);
    expect(notices()).toEqual(["Warning: Verb crashed: reload it to open its window"]);
  });
});

describe("setDeviceWindow (the window button) on a plugin", () => {
  it("shows the opening state until the engine answers, and ignores clicks meanwhile", async () => {
    let answer: (v: unknown) => void = () => {};
    const sent: Command[] = [];
    const t = { send: (c: Command) => (sent.push(c), new Promise((r) => (answer = r))) } as unknown as EngineTransport;
    const d = device("p3", true);
    setDeviceWindow(t, d, true);
    await flush();
    expect(usePluginStore.getState().opening["p3"]).toBe(true);
    setDeviceWindow(t, d, true);
    await flush();
    expect(sent).toHaveLength(1);
    answer({ type: "Unit" });
    await flush();
    expect(usePluginStore.getState().opening["p3"]).toBeUndefined();
    expect(usePluginStore.getState().editors["p3"]).toBe(true);
  });

  it("waits for a plugin still loading before asking for its editor", async () => {
    const t = recorder();
    usePluginStore.setState({ loading: ["p4"] });
    setDeviceWindow(t, device("p4", true), true);
    await flush();
    expect(t.sent).toEqual([]);
    expect(usePluginStore.getState().opening["p4"]).toBe(true);
    usePluginStore.setState({ loading: [] });
    await flush();
    expect(types(t)).toEqual(["OpenEditor"]);
    expect(usePluginStore.getState().editors["p4"]).toBe(true);
  });

  it("says why it couldn't open, in a toast", async () => {
    const t = {
      send: (c: Command) => Promise.reject(new CommandFailedError({ code: "Plugin", message: "the plugin has no editor" }, c)),
    } as unknown as EngineTransport;
    setDeviceWindow(t, { ...device("p5", true), name: "Synth" }, true);
    await flush();
    expect(usePluginStore.getState().opening["p5"]).toBeUndefined();
    expect(usePluginStore.getState().editors["p5"]).toBeUndefined();
    expect(notices()).toEqual(["Error: couldn't open the Synth window: the plugin has no editor"]);
  });
});
