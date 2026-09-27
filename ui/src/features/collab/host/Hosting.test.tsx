/** Hosting UI + wiring (SetHosting, listener list, web sender lifecycle) against MockTransport. */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, Event, ReplyValue } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { MockTransport, TransportProvider, WasmTransport, type EngineTransport, type SendOptions } from "@/transport";
import { MockCollab } from "@/transport/mock/roadmap/collab";
import { PresenceBar } from "..";
import { useCollabStore } from "../store";
import { canSendFromUi, liveTimeline, streamOutputOf } from "./live";
import { createRtpObserver, observerKind } from "./observer";
import { useHostStore } from "./store";
import { createFakePeer, FakeObserver, FakePeerConnection, fakeStream, FakeTimeline } from "./testing";
import { hostEnv } from "./useHosting";

class CollabMock extends MockTransport {
  sent: Command[] = [];
  override async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push(command);
    return super.send(command, opts);
  }
  get sim(): MockCollab {
    return (this as unknown as { collab: MockCollab }).collab;
  }
  emitCollab(event: Event) {
    (this.sim as unknown as { host: { emit(e: Event): void } }).host.emit(event);
  }
  hosting() {
    return this.sent.flatMap((c) => (c.domain === "Collab" && c.command.type === "SetHosting" ? [c.command] : []));
  }
  signals() {
    return this.sent.flatMap((c) => (c.domain === "Collab" && c.command.type === "SendSignal" ? [c.command] : []));
  }
}

const defaults = { ...hostEnv };

async function setup() {
  const mock = new CollabMock({ timers: "manual", seed: 5 });
  render(
    <TransportProvider transport={mock}>
      <PresenceBar />
    </TransportProvider>,
  );
  await waitFor(() => {
    if (!useProjectStore.getState().project) throw new Error("not connected");
  });
  return mock;
}

async function join() {
  fireEvent.click(screen.getByTestId("collab-button"));
  fireEvent.change(screen.getByLabelText("Relay address"), { target: { value: "ws://relay:9003" } });
  fireEvent.change(screen.getByLabelText("Session"), { target: { value: "jam" } });
  fireEvent.change(screen.getByLabelText("Your name"), { target: { value: "Ada" } });
  fireEvent.click(screen.getByRole("button", { name: "Join" }));
  await waitFor(() => expect(screen.getByTestId("collab-button").textContent).toBe("● jam"));
}

afterEach(() => {
  Object.assign(hostEnv, defaults);
  useCollabStore.getState().reset();
  useHostStore.setState({ allow: true, remoteTransport: true, uiSender: false, listeners: [], links: {}, iceServers: [] });
  FakePeerConnection.created = [];
  localStorage.clear();
});

describe("Hosting (mock transport, no UI sender)", () => {
  it("declares the policy on join, lists listeners, and turns hosting off", async () => {
    const mock = await setup();
    expect(mock.hosting()).toEqual([]);
    await join();
    expect(mock.hosting()).toEqual([{ type: "SetHosting", allow: true, ui_sender: false, remote_transport: true }]);
    // The mock peer listens (native endpoint, as if the mock had a native sender).
    expect(await screen.findByTestId("collab-listening-badge")).toHaveTextContent("1 listening");

    fireEvent.click(screen.getByTestId("collab-button"));
    const hosting = screen.getByTestId("collab-hosting");
    expect(screen.getByTestId("collab-listeners")).toHaveTextContent("Mock peer");
    // A native link is "connecting" until the peer's presence says it listens (next test).
    expect(screen.getByTestId("collab-listeners")).toHaveTextContent("connecting…");
    const remote = screen.getByRole("switch", { name: /Listeners can play/ });
    expect(remote).toHaveAttribute("aria-checked", "true");
    fireEvent.click(remote);
    await waitFor(() => expect(mock.hosting().at(-1)).toMatchObject({ allow: true, remote_transport: false }));

    const allow = screen.getByRole("switch", { name: "Let others listen to my computer" });
    fireEvent.click(allow);
    await waitFor(() => expect(mock.hosting().at(-1)).toMatchObject({ allow: false }));
    expect(hosting).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByTestId("collab-listeners")).toBeNull());
    expect(screen.queryByTestId("collab-listening-badge")).toBeNull();
    expect(screen.getByRole("switch", { name: /Listeners can play/ })).toBeDisabled();
    expect(JSON.parse(localStorage.getItem("eth-collab-hosting")!)).toEqual({ allow: false, remoteTransport: false });
  });

  it("shows a native (engine) listener as connecting until its presence says it listens", async () => {
    const mock = await setup();
    await join();
    fireEvent.click(screen.getByTestId("collab-button"));
    // Media is not flowing yet: the peer's presence does not say it listens to us.
    await waitFor(() => expect(screen.getByTestId("collab-listeners")).toHaveTextContent("connecting…"));
    const peer = useCollabStore.getState().peers.find((p) => p.site === "2")!;
    const listening = { ...peer, state: { ...peer.state, listening_to: "1" } };
    act(() => mock.emitCollab({ type: "Collab", event: { type: "Presence", peers: [listening] } }));
    await waitFor(() => expect(screen.getByTestId("collab-listeners")).not.toHaveTextContent("connecting…"));
    expect(screen.getByTestId("collab-listeners")).toHaveTextContent("listening");
  });
});

describe("Hosting (web sender)", () => {
  function webEnv() {
    const observer = new FakeObserver();
    const timeline = new FakeTimeline();
    Object.assign(hostEnv, {
      canSend: () => true,
      streamOutput: () => ({ stream: fakeStream(), context: {} as AudioContext, tapClock: new SharedArrayBuffer(8) }),
      createPeer: createFakePeer,
      createObserver: () => observer,
      timeline: () => timeline,
    });
    return { observer, timeline };
  }

  it("runs one peer connection per Ui listener and routes their signals", async () => {
    const { observer } = webEnv();
    const mock = await setup();
    await join();
    expect(mock.hosting()).toEqual([{ type: "SetHosting", allow: true, ui_sender: true, remote_transport: true }]);
    await waitFor(() => expect(FakePeerConnection.created).toHaveLength(1));
    await waitFor(() => expect(mock.signals()[0]).toMatchObject({ to: "2", stream: 1, signal: { type: "Offer" } }));

    act(() => mock.emitCollab({ type: "Collab", event: { type: "Signal", from: "2", stream: 1, signal: { type: "Answer", sdp: "answer" } } }));
    await waitFor(() => expect(FakePeerConnection.created[0]!.remoteDescription).toEqual({ type: "answer", sdp: "answer" }));
    act(() => FakePeerConnection.created[0]!.setState("connected"));
    fireEvent.click(screen.getByTestId("collab-button"));
    await waitFor(() => expect(screen.getByTestId("collab-listeners")).toHaveTextContent("listening"));

    // The listener leaves: its connection closes.
    act(() => mock.sim.simulateListener("2", null));
    expect(FakePeerConnection.created[0]!.closed).toBe(true);

    // Hosting off: the sender stops.
    act(() => mock.sim.simulateListener("2", 3));
    await waitFor(() => expect(FakePeerConnection.created).toHaveLength(2));
    fireEvent.click(screen.getByRole("switch", { name: "Let others listen to my computer" }));
    await waitFor(() => expect(FakePeerConnection.created[1]!.closed).toBe(true));
    expect(observer.disposed).toBe(true);
  });

  it("warns on the web when the browser cannot stream", async () => {
    Object.assign(hostEnv, { canSend: () => false });
    const mock = await setup();
    Object.defineProperty(mock, "kind", { value: "wasm" });
    await join();
    fireEvent.click(screen.getByTestId("collab-button"));
    expect(screen.getByRole("note")).toHaveTextContent(/can't stream your audio/);
  });
});

describe("capability detection (SetHosting.ui_sender)", () => {
  class Sender {}
  const withStreams = { RTCPeerConnection: class {}, RTCRtpSender: { prototype: { createEncodedStreams() {} } } };

  it("needs WebRTC with encoded transforms", () => {
    expect(observerKind({})).toBeNull();
    expect(observerKind({ RTCPeerConnection: class {}, RTCRtpSender: { prototype: Sender.prototype } })).toBeNull();
    expect(observerKind(withStreams)).toBe("streams");
    expect(observerKind({ ...withStreams, RTCRtpScriptTransform: class {} })).toBe("script");
    expect(createRtpObserver(withStreams)?.peerConfig).toEqual({ encodedInsertableStreams: true });
    expect(createRtpObserver({ RTCPeerConnection: class {}, RTCRtpScriptTransform: class {} })?.peerConfig).toEqual({});
    expect(createRtpObserver({})).toBeNull();
  });

  it("needs a transport exposing the stream tap", () => {
    const out = { stream: fakeStream(), context: {} as AudioContext, tapClock: new SharedArrayBuffer(8) };
    const mock = new MockTransport({ timers: "manual" });
    const wasm = Object.assign(Object.create(mock) as EngineTransport, { streamOutput: () => out });
    const notStarted = Object.assign(Object.create(mock) as EngineTransport, { streamOutput: () => null });
    expect(streamOutputOf(mock)).toBeNull();
    expect(streamOutputOf(wasm)).toBe(out);
    expect(canSendFromUi(mock, withStreams)).toBe(false);
    expect(canSendFromUi(notStarted, withStreams)).toBe(false);
    expect(canSendFromUi(wasm, withStreams)).toBe(true);
    expect(canSendFromUi(wasm, {})).toBe(false);
    mock.dispose();
  });

  it("WasmTransport.streamOutput delegates to its endpoint", () => {
    const out = { stream: fakeStream(), context: {} as AudioContext, tapClock: new SharedArrayBuffer(8) };
    const endpoint = {
      start: () => Promise.resolve(),
      post: () => undefined,
      onMessages: () => () => undefined,
      onFatal: () => () => undefined,
      dispose: () => undefined,
    };
    expect(new WasmTransport().streamOutput()).toBeNull();
    expect(new WasmTransport({ endpoint }).streamOutput()).toBeNull();
    const t = new WasmTransport({ endpoint: { ...endpoint, streamOutput: () => out } });
    expect(t.streamOutput()).toBe(out);
    expect(streamOutputOf(t)).toBe(out);
    t.dispose();
    expect(t.streamOutput()).toBeNull();
  });

  it("maps AudioContext timing to encoded frame starts", () => {
    const context = {
      sampleRate: 48_000,
      outputLatency: 0.02,
      baseLatency: 0.01,
      getOutputTimestamp: () => ({ contextTime: 2, performanceTime: 1_000 }),
    } as unknown as AudioContext;
    const t = liveTimeline({ stream: fakeStream(), context, tapClock: new SharedArrayBuffer(128) });
    expect(t.read()).toBeNull();
    // 2 s + 0.1 s + 30 ms latency - 30 ms encoder delay = 2.1 s.
    expect(t.frameStart(1_100)).toBeCloseTo(2.1 * 48_000, 6);
    const idle = liveTimeline({ stream: fakeStream(), context: { ...context, getOutputTimestamp: () => ({}) } as AudioContext, tapClock: new SharedArrayBuffer(128) });
    expect(idle.frameStart(1_100)).toBeNull();
  });
});
