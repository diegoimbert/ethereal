import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Device, DeviceId, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { TransportProvider } from "@/transport";
import { PluginLoadingStatus, TrackPluginLoading } from "./index";
import { usePluginStore } from "./pluginStore";
import { DesktopFake, VERB } from "./testing";

let transport: DesktopFake | undefined;
afterEach(() => {
  cleanup();
  transport?.dispose();
  transport = undefined;
  useProjectStore.getState().reset();
  usePluginStore.getState().reset();
});

const plugin = (id: string, track: TrackId): Device =>
  ({
    id: id as DeviceId,
    track,
    order: id,
    name: "Verb",
    enabled: true,
    kind: { type: "Plugin", plugin: { format: "Clap", plugin_id: VERB.id, name: "Verb", vendor: "Acme", version: "1", sandboxed: false, state: null } },
    pad: null,
    chain: null,
  }) as unknown as Device;

describe("plugins loading", () => {
  it("shows each track's share of loaded plugins, and the project's in the status bar, until all are loaded", async () => {
    transport = new DesktopFake();
    render(
      <TransportProvider transport={transport}>
        <TrackPluginLoading track={"T1" as TrackId} />
        <TrackPluginLoading track={"T2" as TrackId} />
        <PluginLoadingStatus />
      </TransportProvider>,
    );
    await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
    act(() =>
      useProjectStore.setState((s) => ({
        project: {
          ...s.project!,
          devices: { ...s.project!.devices, A: plugin("A", "T1" as TrackId), B: plugin("B", "T1" as TrackId), C: plugin("C", "T2" as TrackId) },
        },
      })),
    );
    expect(screen.queryByRole("progressbar")).toBeNull();

    act(() => transport!.emit({ type: "Loading", pending: ["B", "C"] as DeviceId[] }));
    expect(screen.getByRole("progressbar", { name: "loading plugins: 1 of 2" })).toHaveAttribute("aria-valuenow", "1");
    expect(screen.getByRole("progressbar", { name: "loading plugins: 0 of 1" })).toBeTruthy();
    expect(screen.getByTestId("plugins-loading").textContent).toBe("loading plugins 1 / 3");

    // T1's are all loaded: its bar goes; then everything is.
    act(() => transport!.emit({ type: "Loading", pending: ["C"] as DeviceId[] }));
    expect(screen.getAllByRole("progressbar")).toHaveLength(1);
    act(() => transport!.emit({ type: "Loading", pending: [] }));
    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(screen.queryByTestId("plugins-loading")).toBeNull();
  });
});
