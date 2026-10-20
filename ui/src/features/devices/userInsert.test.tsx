import { act, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Command, ReplyValue, Track } from "@/generated";
import { useNotices } from "@/features/notifications";
import { flush, renderWithMock, resetStores, store, stubPointerCapture } from "@/features/devices/testUtils";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { cmd, newId, type EngineTransport } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { MOCK_MEGA_PLUGIN_ID } from "@/transport/mock/plugins";
import { insertFromLibrary, insertedDeviceId, sendUserInsert } from "./userInsert";
import { DeviceWindowsRoot, resetDeviceWindows } from "./window";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  resetStores(mock);
  mock = undefined;
  resetDeviceWindows();
  usePluginStore.getState().reset();
  useNotices.getState().clear();
});

const trackOfKind = (kind: Track["kind"]): Track => Object.values(store().project!.tracks).find((t) => t.kind === kind)!;
const messages = () => useNotices.getState().notices.map((n) => `${n.level}: ${n.message}`);

/** `mock`, recording what is sent; the plugin host's editor commands succeed. */
function host(base: MockTransport, fail?: Error): EngineTransport & { sent: Command[] } {
  const sent: Command[] = [];
  return Object.assign(Object.create(base) as EngineTransport, {
    sent,
    async send(command: Command): Promise<ReplyValue> {
      sent.push(command);
      if (fail) throw fail;
      if (command.domain === "Plugin") return { type: "Unit" };
      return base.send(command);
    },
  });
}

describe("a device the user adds", () => {
  it("a factory device from the library opens its window", async () => {
    mock = await renderWithMock(<DeviceWindowsRoot />);
    const track = trackOfKind("Audio");
    await act(() => insertFromLibrary(mock!, track, { name: "EQ", category: "AudioEffect", device: { type: "Builtin", builtin: "Eq" } }));
    await flush();
    expect(await screen.findByRole("dialog", { name: "eq window" })).toBeInTheDocument();
    expect(messages()).toEqual([]);
  });

  it("a plugin opens its editor window", async () => {
    mock = await renderWithMock(<DeviceWindowsRoot />);
    const t = host(mock);
    const track = trackOfKind("Audio");
    const id = newId();
    const insert = cmd("Device", {
      type: "Insert",
      id,
      track: track.id,
      device: { type: "Plugin", plugin_id: MOCK_MEGA_PLUGIN_ID, format: "Clap", sandboxed: null },
      before: null,
    });
    await act(() => sendUserInsert(t, insert, "Mock Mega"));
    await waitFor(() => expect(t.sent).toContainEqual(cmd("Plugin", { type: "OpenEditor", device: id })));
    await waitFor(() => expect(usePluginStore.getState().editors[id]).toBeTruthy());
    // Not a factory window.
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("an instrument on an audio track says why instead of doing nothing", async () => {
    mock = await renderWithMock(<DeviceWindowsRoot />);
    const t = host(mock);
    const added = await insertFromLibrary(t, trackOfKind("Audio"), {
      name: "Surge XT",
      category: "Instrument",
      device: { type: "Plugin", plugin_id: "org.surge-synth-team.surge-xt", format: "Clap" },
    });
    expect(added).toBe(false);
    expect(t.sent).toEqual([]);
    expect(messages()).toEqual(["Warning: Surge XT is an instrument: select a midi track"]);
  });

  it("a failed insert shows the error", async () => {
    mock = await renderWithMock(<DeviceWindowsRoot />);
    const t = host(mock, new Error("plugin failed to load"));
    const track = trackOfKind("Midi");
    const insert = cmd("Device", {
      type: "Insert",
      id: newId(),
      track: track.id,
      device: { type: "Plugin", plugin_id: "x", format: "Clap", sandboxed: null },
      before: null,
    });
    expect(await sendUserInsert(t, insert, "Surge XT")).toBe(false);
    expect(messages()).toHaveLength(1);
    expect(messages()[0]).toMatch(/^Error: couldn't add Surge XT: .*plugin failed to load/);
  });

  it("finds the new device in a Replace Instrument batch", () => {
    const insert = cmd("Device", { type: "Insert", id: "new", track: "t", device: { type: "Builtin", device: { type: "Synth" } }, before: "old" });
    expect(
      insertedDeviceId(cmd("Edit", { type: "Batch", label: "Replace Instrument", commands: [insert, cmd("Device", { type: "Remove", id: "old" })] })),
    ).toBe("new");
    expect(insertedDeviceId(cmd("Device", { type: "Remove", id: "old" }))).toBeNull();
  });
});
