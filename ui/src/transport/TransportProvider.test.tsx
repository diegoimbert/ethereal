import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useContext, useEffect } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { usePlayheadPosition } from "@/state/playhead";
import { useProjectStore } from "@/state/projectStore";
import { useTracksOrdered } from "@/state/selectors";
import { cmd } from "./cmd";
import { TransportContext, useConnectionStatus, useTransport } from "./context";
import { MockTransport } from "./mock/MockTransport";
import { TransportProvider } from "./TransportProvider";
import { useTransportSwitch } from "./createDefaultTransport";
import { WasmTransport } from "./wasm/WasmTransport";

function Probe() {
  const transport = useTransport();
  const { status } = useConnectionStatus();
  const tracks = useTracksOrdered();
  const position = usePlayheadPosition();
  return (
    <div>
      <span data-testid="status">{status}</span>
      <span data-testid="kind">{transport.kind}</span>
      <span data-testid="tracks">{tracks.map((t) => t.name).join(",")}</span>
      <span data-testid="position">{position.toFixed(2)}</span>
    </div>
  );
}

let mock: MockTransport | undefined;
afterEach(() => {
  mock?.dispose();
  useProjectStore.getState().reset();
});

function RetryProbe() {
  const ctx = useContext(TransportContext)!;
  return (
    <div>
      <span data-testid="status">{ctx.connection.status}</span>
      <button type="button" onClick={ctx.retry}>
        retry
      </button>
    </div>
  );
}

describe("TransportProvider retry (#597)", () => {
  it("connects again after a failed connect", async () => {
    mock = new MockTransport({ timers: "manual" });
    const connect = mock.connect.bind(mock);
    let attempts = 0;
    mock.connect = () => (++attempts === 1 ? Promise.reject(new Error("no AudioWorklet")) : connect());
    render(
      <TransportProvider transport={mock}>
        <RetryProbe />
      </TransportProvider>,
    );
    await waitFor(() => expect(screen.getByTestId("status").textContent).toBe("error"));
    fireEvent.click(screen.getByRole("button", { name: "retry" }));
    await waitFor(() => expect(screen.getByTestId("status").textContent).toBe("connected"));
    expect(attempts).toBe(2);
  });
});

describe("TransportProvider", () => {
  it("connects, mirrors patches and forwards the playhead", async () => {
    mock = new MockTransport({ timers: "manual" });
    render(
      <TransportProvider transport={mock}>
        <Probe />
      </TransportProvider>,
    );
    await waitFor(() => expect(screen.getByTestId("status").textContent).toBe("connected"));
    expect(screen.getByTestId("kind").textContent).toBe("mock");
    expect(screen.getByTestId("tracks").textContent).toBe("Keys,Bass,Drums,A Delay,master");

    const keys = Object.values(useProjectStore.getState().project!.tracks).find((t) => t.name === "Keys")!;
    await act(() => mock!.send(cmd("Track", { type: "Rename", id: keys.id, name: "Piano" })));
    expect(screen.getByTestId("tracks").textContent).toBe("Piano,Bass,Drums,A Delay,master");

    await act(async () => {
      await mock!.send(cmd("Transport", { type: "Play" }));
      mock!.tick(1000);
    });
    expect(screen.getByTestId("position").textContent).toBe("2.00");
  });

  it("after switching engines and back, undo reflects the engine's history; dirty and armed don't carry over (#598)", async () => {
    mock = new MockTransport({ timers: "manual", seed: 1 });
    const other = new MockTransport({ timers: "manual", seed: 2 });
    const box: { sw: ReturnType<typeof useTransportSwitch> } = { sw: null };
    const Switch = () => {
      const sw = useTransportSwitch();
      useEffect(() => {
        box.sw = sw;
      });
      return null;
    };
    render(
      <TransportProvider transport={mock}>
        <Probe />
        <Switch />
      </TransportProvider>,
    );
    await waitFor(() => expect(screen.getByTestId("status").textContent).toBe("connected"));
    const store = useProjectStore.getState;
    const keys = Object.values(store().project!.tracks).find((t) => t.name === "Keys")!;
    await act(() => mock!.send(cmd("Track", { type: "Rename", id: keys.id, name: "Piano" })));
    expect(store().history.can_undo).toBe(true);

    // To engine B (the same demo, unedited, no history), which reports nothing about dirty or
    // armed tracks.
    act(() => {
      store().setDirty(true);
      store().setArmedTracks([keys.id]);
      box.sw!.switchToRemote(other);
    });
    await waitFor(() => expect(screen.getByTestId("kind").textContent).toBe("mock"));
    await waitFor(() => expect(screen.getByTestId("status").textContent).toBe("connected"));
    await waitFor(() => expect(screen.getByTestId("tracks").textContent).toContain("Keys"));
    expect(store().history.can_undo).toBe(false);
    expect(store().dirty).toBe(false);
    expect(store().armedTracks).toEqual([]);

    // Back to A: its undo stack is still there, and the button says so.
    act(() => box.sw!.switchToLocal());
    await waitFor(() => expect(screen.getByTestId("tracks").textContent).toContain("Piano"));
    await waitFor(() => expect(store().history.can_undo).toBe(true));
    expect(store().history.undo_label).not.toBeNull();
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(screen.getByTestId("tracks").textContent).toContain("Keys");
  });

  it("reports a connection error for unimplemented transports", async () => {
    render(
      <TransportProvider transport={new WasmTransport()}>
        <Probe />
      </TransportProvider>,
    );
    await waitFor(() => expect(screen.getByTestId("status").textContent).toBe("error"));
  });
});
