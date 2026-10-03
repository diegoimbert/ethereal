import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClientHello, ClientMessage, ServerMessage } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { cmd, CommandFailedError, MockTransport, TransportProvider, useTransport } from "@/transport";
import { ConnectDialog, RemoteEngineSettings, useUploadDrop } from ".";

/**
 * A WebSocket whose "server" is a MockTransport (the remote engine): hello with token
 * "tok", then ClientMessages go to the mock and its events/replies come back as frames.
 */
let remoteEngine: MockTransport;
let sockets: FakeServerSocket[] = [];

class FakeServerSocket {
  binaryType = "blob";
  readyState = 0;
  onopen: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: ((ev: { code: number; reason: string }) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  private helloDone = false;
  private off: (() => void) | null = null;

  constructor(readonly url: string) {
    sockets.push(this);
    setTimeout(() => {
      this.readyState = 1;
      this.onopen?.({});
    }, 0);
  }

  private deliver(m: ServerMessage | object) {
    setTimeout(() => this.onmessage?.({ data: JSON.stringify(m) }), 0);
  }

  send(data: string) {
    if (!this.helloDone) {
      const hello = JSON.parse(data) as ClientHello;
      this.helloDone = true;
      if (hello.token !== "tok") {
        this.deliver({ type: "Rejected", reason: "BadToken", message: "invalid token" });
        setTimeout(() => this.close(4001, "invalid token"), 0);
        return;
      }
      this.off = remoteEngine.onEvent((body) => this.deliver({ kind: "Event", body }));
      this.deliver({
        type: "Welcome",
        session: "s",
        server: {
          name: "studio",
          app_version: "0.1.0",
          protocol_version: 1,
          instance: "test",
          auth_required: true,
          capabilities: { plugins: true, recording: true, upload: true, export: true, collab: false },
        },
      });
      return;
    }
    const m = JSON.parse(data) as ClientMessage;
    remoteEngine.send(m.command, { gesture: m.gesture ?? undefined }).then(
      (value) => this.deliver({ kind: "Reply", body: { id: m.id, result: { status: "Ok", value } } }),
      (e: unknown) => this.deliver({ kind: "Reply", body: { id: m.id, result: { status: "Err", error: (e as CommandFailedError).error } } }),
    );
  }

  close(code = 1000, reason = "") {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.off?.();
    this.onclose?.({ code, reason });
  }
}

function Probe() {
  const t = useTransport();
  const drop = useUploadDrop();
  return (
    <span data-testid="probe" data-kind={t.kind} data-droppable={drop.onDrop ? "yes" : "no"}>
      {useProjectStore((s) => s.project?.settings.name)}
    </span>
  );
}

let local: MockTransport;

beforeEach(async () => {
  sockets = [];
  localStorage.clear();
  vi.stubGlobal("WebSocket", FakeServerSocket);
  local = new MockTransport({ timers: "manual" });
  remoteEngine = new MockTransport({ timers: "manual" });
  const p = await remoteEngine.connect();
  await remoteEngine.send(cmd("Project", { type: "Rename", id: p.id, name: "On the server" }));
});

afterEach(() => {
  vi.unstubAllGlobals();
  local.dispose();
  remoteEngine.dispose();
  useProjectStore.getState().reset();
});

function renderApp() {
  render(
    <TransportProvider transport={local}>
      <ConnectDialog />
      <RemoteEngineSettings />
      <Probe />
    </TransportProvider>,
  );
}

/** Settings > Advanced > Engine server (base-115 moved the form out of the top bar). */
async function connectWith(url: string, token: string) {
  expect(screen.queryByTestId("remote-button")).toBeNull();
  fireEvent.change(screen.getByLabelText("Server address"), { target: { value: url } });
  fireEvent.change(screen.getByLabelText("Token"), { target: { value: token } });
  fireEvent.click(screen.getByRole("button", { name: "Connect" }));
}

describe("ConnectDialog", () => {
  it("connects to a remote engine, switches the UI to it and back", async () => {
    renderApp();
    await waitFor(() => expect(screen.getByTestId("probe").textContent).not.toBe(""));
    const localName = screen.getByTestId("probe").textContent;
    expect(screen.getByTestId("probe").dataset.droppable).toBe("no");

    await connectWith("studio.local:9000", "tok");
    await waitFor(() => expect(screen.getByTestId("probe").textContent).toBe("On the server"));
    expect(sockets[0]!.url).toBe("ws://studio.local:9000/");
    expect(screen.getByTestId("probe").dataset.kind).toBe("remote");
    expect(screen.getByTestId("probe").dataset.droppable).toBe("yes");
    expect(screen.getByTestId("remote-button").textContent).toContain("studio");
    expect(screen.getByTestId("settings-engine-server").textContent).toMatch(/Connected to studio/);
    expect(localStorage.getItem("eth-remote-url")).toBe("studio.local:9000");
    expect(JSON.stringify(localStorage)).not.toContain("tok");

    // Edits go to the remote engine and come back as patches.
    const project = useProjectStore.getState().project!;
    await act(() => remoteEngine.send(cmd("Project", { type: "Rename", id: project.id, name: "Renamed remotely" })).then(() => undefined));
    await waitFor(() => expect(screen.getByTestId("probe").textContent).toBe("Renamed remotely"));

    // Disconnect from the top-bar indicator's dialog: back to the local engine.
    fireEvent.click(screen.getByTestId("remote-button"));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Disconnect" }));
    await waitFor(() => expect(screen.getByTestId("probe").dataset.kind).toBe("mock"));
    await waitFor(() => expect(screen.getByTestId("probe").textContent).toBe(localName));
    expect(sockets[0]!.readyState).toBe(3);
  });

  it("shows why a connection was refused and stays local", async () => {
    renderApp();
    await connectWith("ws://studio:1/", "nope");
    expect(await screen.findByText("Wrong or missing token.")).toBeTruthy();
    expect(screen.getByTestId("probe").dataset.kind).toBe("mock");
    fireEvent.change(screen.getByLabelText("Server address"), { target: { value: "http://x" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByText(/Enter the engine server address/)).toBeTruthy();
  });

  it("returns to the local engine when the connection drops", async () => {
    renderApp();
    await connectWith("ws://studio:1/", "tok");
    await waitFor(() => expect(screen.getByTestId("probe").dataset.kind).toBe("remote"));
    act(() => sockets[0]!.close(1006, ""));
    // Falling back boots a fresh local engine, which can exceed waitFor's 1 s default on a
    // loaded machine (seen on the shared Linux devbox during the full suite).
    await waitFor(() => expect(screen.getByTestId("probe").dataset.kind).toBe("mock"), {
      timeout: 5000,
    });
    expect(screen.getByRole("alert").textContent).toMatch(/Connection to studio lost/);
  });
});
