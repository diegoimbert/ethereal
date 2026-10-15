import { act, cleanup, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { resetArrangementUi } from "@/features/arrangement/uiStore";
import { useProjectStore } from "@/state";
import { resetShell } from "@/state/shell";
import { App } from "@/app/App";
import { CommandFailedError, MockTransport, type EngineTransport } from "@/transport";
import { unsupportedTooltip } from "@/features/recording/support";
import { bindBackgroundStop } from "./background";
import { enableMobileUi, formFactorOf } from "./mode";
import { openSheet } from "./sheets";

describe("formFactorOf", () => {
  it("phones by their short side, portrait or landscape by the longer axis", () => {
    expect(formFactorOf(390, 844)).toBe("phone-portrait");
    expect(formFactorOf(844, 390)).toBe("phone-landscape");
    expect(formFactorOf(412, 915)).toBe("phone-portrait");
    // A square-ish small window is a portrait phone.
    expect(formFactorOf(500, 500)).toBe("phone-portrait");
  });

  it("tablets from 600 px on the short side, in both orientations", () => {
    expect(formFactorOf(599, 1000)).toBe("phone-portrait");
    expect(formFactorOf(600, 1000)).toBe("tablet");
    expect(formFactorOf(820, 1180)).toBe("tablet");
    expect(formFactorOf(1180, 820)).toBe("tablet");
    expect(formFactorOf(744, 1133)).toBe("tablet");
  });
});

describe("bindBackgroundStop", () => {
  const fakeDoc = () => {
    const target = new EventTarget();
    const doc = Object.assign(target, { visibilityState: "visible" as DocumentVisibilityState });
    return doc;
  };
  const transportStub = () => {
    const send = vi.fn((_c: unknown) => Promise.resolve({ type: "Ok" }));
    return { send, transport: { send } as unknown as EngineTransport };
  };
  afterEach(() => resetStores());

  const setTransport = (playing: boolean, recording: boolean) =>
    useProjectStore.setState({ transport: { ...(useProjectStore.getState().transport ?? ({} as never)), playing, recording } as never });

  it("stops playback and finishes a take when the page is hidden", () => {
    const doc = fakeDoc();
    const win = new EventTarget();
    const { send, transport } = transportStub();
    const off = bindBackgroundStop(transport, doc as never, win as never);
    setTransport(true, true);
    doc.visibilityState = "hidden";
    doc.dispatchEvent(new Event("visibilitychange"));
    expect(send.mock.calls.map((c) => c[0])).toEqual([
      { domain: "Recording", command: { type: "SetRecording", enabled: false } },
      { domain: "Transport", command: { type: "Stop" } },
    ]);
    off();
  });

  it("does nothing when stopped, when becoming visible, or once unbound", () => {
    const doc = fakeDoc();
    const win = new EventTarget();
    const { send, transport } = transportStub();
    const off = bindBackgroundStop(transport, doc as never, win as never);
    setTransport(false, false);
    doc.visibilityState = "hidden";
    doc.dispatchEvent(new Event("visibilitychange"));
    setTransport(true, false);
    doc.visibilityState = "visible";
    doc.dispatchEvent(new Event("visibilitychange"));
    expect(send).not.toHaveBeenCalled();
    // pagehide (iOS closing or freezing the tab) stops too.
    win.dispatchEvent(new Event("pagehide"));
    expect(send).toHaveBeenCalledTimes(1);
    off();
    win.dispatchEvent(new Event("pagehide"));
    expect(send).toHaveBeenCalledTimes(1);
  });
});

describe("MobileShell", () => {
  beforeEach(() => {
    resetShell();
    resetArrangementUi();
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    enableMobileUi();
  });
  afterEach(() => {
    cleanup();
    enableMobileUi(false);
    resetStores();
    vi.restoreAllMocks();
    delete document.documentElement.dataset.ethMobile;
  });

  it("replaces the desktop shell: arrangement, top bar, bottom transport; no rail or panes", async () => {
    await renderWithMock(<App />);
    const shell = await screen.findByTestId("mobile-shell");
    expect(shell.querySelector('[data-slot="main"] [data-feature="arrangement"]')).not.toBeNull();
    expect(screen.getByTestId("mobile-transport")).toBeTruthy();
    expect(document.querySelector(".eth-rail")).toBeNull();
    expect(document.querySelector("[data-pane]")).toBeNull();
    // The layout follows the viewport (jsdom: 1024 × 768, a tablet) and marks the document.
    expect(shell.dataset.layout).toBe("tablet");
    expect(document.documentElement.dataset.ethMobile).toBe("tablet");
  });

  it("the project menu's export audio… opens the export dialog (#572)", async () => {
    await renderWithMock(<App />);
    await screen.findByTestId("mobile-shell");
    fireEvent.click(screen.getByRole("button", { name: "projects" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: /export audio/i }));
    expect(await screen.findByRole("dialog", { name: /export audio/i })).toBeTruthy();
  });

  it("Escape closes the track sheet, wherever the focus is (#640)", async () => {
    await renderWithMock(<App />);
    await screen.findByTestId("mobile-shell");
    const track = Object.values(useProjectStore.getState().project!.tracks).find((t) => t.kind === "Midi")!;
    // Opened from a track header (the focus stays in the arrangement).
    const arr = document.querySelector<HTMLElement>(".eth-arr")!;
    act(() => arr.focus());
    act(() => openSheet({ kind: "track", track: track.id }));
    expect(await screen.findByTestId("track-sheet")).toBeTruthy();
    fireEvent.keyDown(document.activeElement ?? arr, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("track-sheet")).toBeNull());
  });

  it("the layout follows rotation", async () => {
    await renderWithMock(<App />);
    const shell = await screen.findByTestId("mobile-shell");
    const resize = (w: number, h: number) =>
      act(() => {
        Object.assign(window, { innerWidth: w, innerHeight: h });
        window.dispatchEvent(new Event("resize"));
      });
    resize(390, 844);
    expect(shell.dataset.layout).toBe("phone-portrait");
    resize(844, 390);
    expect(shell.dataset.layout).toBe("phone-landscape");
    expect(document.documentElement.dataset.ethMobile).toBe("phone-landscape");
    resize(1024, 768);
  });

  it("the record button is disabled where the host can't record (the browser, #717)", async () => {
    // The web engine: `ListInputs` is unsupported.
    const send = MockTransport.prototype.send;
    vi.spyOn(MockTransport.prototype, "send").mockImplementation(function (this: MockTransport, c, o) {
      if (c.domain === "Recording" && c.command.type === "ListInputs") {
        return Promise.reject(new CommandFailedError({ code: "Unsupported", message: "no inputs in the browser" }, c));
      }
      return send.call(this, c, o);
    });
    await renderWithMock(<App />);
    await screen.findByTestId("mobile-shell");
    const record = screen.getByRole("button", { name: "record" });
    await waitFor(() => expect(record).toBeDisabled());
    expect(record).toHaveAttribute("title", unsupportedTooltip());
    // Play still works.
    expect(screen.getByRole("button", { name: "play" })).toBeEnabled();
  });

  it("the record button records where the host can", async () => {
    const { mock: transport } = await renderWithMock(<App />);
    await screen.findByTestId("mobile-shell");
    const record = screen.getByRole("button", { name: "record" });
    await waitFor(() => expect(record).toBeEnabled());
    const send = vi.spyOn(transport, "send");
    fireEvent.click(record);
    expect(send.mock.calls.map((c) => JSON.stringify(c[0]))).toContain(
      JSON.stringify({ domain: "Recording", command: { type: "SetRecording", enabled: true } }),
    );
  });

  it("play, loop and metronome send the transport's commands", async () => {
    const { mock: transport } = await renderWithMock(<App />);
    await screen.findByTestId("mobile-shell");
    const send = vi.spyOn(transport, "send");
    await waitFor(() => expect(screen.getByRole("button", { name: "play" })).not.toBeDisabled());
    fireEvent.click(screen.getByRole("button", { name: "play" }));
    fireEvent.click(screen.getByRole("button", { name: "loop" }));
    fireEvent.click(screen.getByRole("button", { name: "metronome" }));
    const sent = send.mock.calls.map((c) => JSON.stringify(c[0]));
    expect(sent).toContain(JSON.stringify({ domain: "Transport", command: { type: "Play" } }));
    expect(sent.some((s) => s.includes("SetLoopEnabled"))).toBe(true);
    expect(sent.some((s) => s.includes("SetMetronome"))).toBe(true);
  });
});
