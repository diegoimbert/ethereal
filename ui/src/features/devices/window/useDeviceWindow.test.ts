import { afterEach, describe, expect, it } from "vitest";
import type { Command, Device } from "@/generated";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { flush } from "@/features/devices/testUtils";
import type { EngineTransport } from "@/transport";
import { resetDeviceWindows, useDeviceWindows } from "./store";
import { showDeviceWindow } from "./useDeviceWindow";

/** A transport that records what it is sent (plugin commands succeed). */
function recorder(): EngineTransport & { sent: Command[] } {
  const sent: Command[] = [];
  return { sent, send: (c: Command) => (sent.push(c), Promise.resolve({ type: "Unit" })) } as unknown as EngineTransport & { sent: Command[] };
}
const device = (id: string, plugin: boolean) =>
  ({ id, kind: plugin ? { type: "Plugin", plugin: { id: "p" } } : { type: "Builtin", device: { type: "Eq" } } }) as unknown as Device;

afterEach(() => {
  resetDeviceWindows();
  usePluginStore.setState({ editors: {}, crashed: {} });
});

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

  it("leaves a crashed plugin alone", () => {
    const t = recorder();
    usePluginStore.setState({ crashed: { p2: "boom" } });
    showDeviceWindow(t, device("p2", true));
    expect(t.sent).toEqual([]);
  });
});
