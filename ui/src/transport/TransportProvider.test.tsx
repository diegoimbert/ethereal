import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { usePlayheadPosition } from "@/state/playhead";
import { useProjectStore } from "@/state/projectStore";
import { useTracksOrdered } from "@/state/selectors";
import { cmd } from "./cmd";
import { useConnectionStatus, useTransport } from "./context";
import { MockTransport } from "./mock/MockTransport";
import { TransportProvider } from "./TransportProvider";
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
    expect(screen.getByTestId("tracks").textContent).toBe("Keys,Bass,Drums,A Delay,Master");

    const keys = Object.values(useProjectStore.getState().project!.tracks).find((t) => t.name === "Keys")!;
    await act(() => mock!.send(cmd("Track", { type: "Rename", id: keys.id, name: "Piano" })));
    expect(screen.getByTestId("tracks").textContent).toBe("Piano,Bass,Drums,A Delay,Master");

    await act(async () => {
      await mock!.send(cmd("Transport", { type: "Play" }));
      mock!.tick(1000);
    });
    expect(screen.getByTestId("position").textContent).toBe("2.00");
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
