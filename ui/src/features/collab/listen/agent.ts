// The listener side of "Listen on <peer>" in the UI (docs/COLLAB.md §9): reacts to the
// controller's collab events, runs the WebRTC receiver of the current stream, and drives
// the playhead from the host's stream clock at display rate (§9.4).
import type { CollabCommand, Event, PlayheadFrame, SiteId } from "@/generated";
import { StreamClockMapper, type TempoLike } from "./clock";
import { createAudioSink, ListenReceiver, webrtcUnsupportedReason, type AudioSink, type ReceiverOptions } from "./receiver";
import { activeHost, useListenStore } from "./store";

type Send = (c: CollabCommand) => Promise<unknown>;

export interface AgentDeps {
  send: Send;
  /** The replicated tempo map (`null`: use the anchors' bpm). */
  tempo(): (TempoLike & { beatsToSeconds(b: number): number }) | null;
  /** Show `frame` instead of the engine's playhead; `null` = back to the engine's. */
  setPlayhead(frame: PlayheadFrame | null): void;
  createReceiver?: (o: ReceiverOptions) => ListenReceiver;
  requestFrame?: (cb: () => void) => number;
  cancelFrame?: (id: number) => void;
  /** Local clock for anchor arrivals (ms). */
  now?: () => number;
}

/** The sink created in the last "Listen" click, for the receiver that follows. */
let preparedSink: AudioSink | null = null;

const describe = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * "Listen on <host>'s computer": call from the click handler (the audio output is created
 * in the gesture, for autoplay).
 */
export async function listenTo(send: Send, host: SiteId, name = ""): Promise<void> {
  const unsupported = webrtcUnsupportedReason();
  if (unsupported) {
    useListenStore.setState({ error: `Cannot listen: ${unsupported}` });
    return;
  }
  preparedSink?.close();
  const sink = createAudioSink();
  preparedSink = sink;
  useListenStore.setState({ error: null, hostName: [host, name] });
  try {
    await send({ type: "Listen", host });
  } catch (e) {
    if (preparedSink === sink) preparedSink = null;
    sink.close();
    useListenStore.setState({ error: `Cannot listen: ${describe(e)}` });
  }
}

export async function stopListening(send: Send): Promise<void> {
  try {
    await send({ type: "StopListening" });
  } catch (e) {
    useListenStore.setState({ error: describe(e) });
  }
}

export class ListenAgent {
  private receiver: ListenReceiver | null = null;
  private readonly mapper = new StreamClockMapper();
  private frameId: number | null = null;
  private overriding = false;
  private readonly now: () => number;

  constructor(private readonly deps: AgentDeps) {
    this.now = deps.now ?? (() => performance.now());
  }

  onEvent(event: Event): void {
    if (event.type !== "Collab") return;
    const e = event.event;
    switch (e.type) {
      case "IceServers":
        useListenStore.setState({ iceServers: e.servers });
        break;
      case "ListenStatus":
        useListenStore.setState({
          listening: e.status.listening,
          listeners: e.status.listeners,
        });
        this.reconcile();
        break;
      case "Signal":
        if (this.receiver && this.receiver.host === e.from && this.receiver.stream === e.stream) void this.receiver.onSignal(e.signal);
        break;
      case "StreamClock":
        if (this.receiver && this.receiver.host === e.from && this.receiver.stream === e.stream) this.mapper.push(e.clock, this.now());
        break;
      case "Session":
        if (e.status.type === "Offline") useListenStore.setState({ listeners: [], countIn: false });
        break;
    }
  }

  /** Open/close the receiver to match the controller's listen state. */
  private reconcile(): void {
    const l = useListenStore.getState().listening;
    const host = activeHost(l);
    if (host !== null && (l.type === "Connecting" || l.type === "Listening")) {
      if (this.receiver && !this.receiver.isClosed && this.receiver.host === host && this.receiver.stream === l.stream) return;
      this.closeReceiver();
      const sink = preparedSink ?? createAudioSink();
      preparedSink = null;
      const stream = l.stream;
      const options: ReceiverOptions = {
        host,
        stream,
        iceServers: useListenStore.getState().iceServers,
        sink,
        send: (signal) => void this.deps.send({ type: "SendSignal", to: host, stream, signal }).catch(() => undefined),
        onFailed: (reason) => useListenStore.setState({ error: reason }),
      };
      this.receiver = this.deps.createReceiver ? this.deps.createReceiver(options) : new ListenReceiver(options);
      this.startLoop();
    } else {
      this.closeReceiver();
    }
  }

  private closeReceiver(): void {
    this.receiver?.close();
    this.receiver = null;
    this.mapper.reset();
    this.stopLoop();
    if (this.overriding) {
      this.overriding = false;
      this.deps.setPlayhead(null);
    }
    if (useListenStore.getState().countIn) useListenStore.setState({ countIn: false });
  }

  private startLoop(): void {
    const request = this.deps.requestFrame ?? ((cb) => requestAnimationFrame(cb));
    const loop = () => {
      this.frame();
      this.frameId = this.receiver ? request(loop) : null;
    };
    if (this.frameId === null) this.frameId = request(loop);
  }

  private stopLoop(): void {
    if (this.frameId === null) return;
    (this.deps.cancelFrame ?? ((id) => cancelAnimationFrame(id)))(this.frameId);
    this.frameId = null;
  }

  /** One display frame: map what is heard to the host's timeline (§9.4). */
  frame(): void {
    const r = this.receiver;
    if (!r) return;
    const tempo = this.deps.tempo();
    const rtp = r.playoutRtp();
    const m = rtp !== null ? this.mapper.map(rtp, tempo) : this.mapper.mapByArrival(this.now(), r.receiveDelaySec(), tempo);
    if (!m) return;
    this.overriding = true;
    this.deps.setPlayhead({
      transport: {
        position: m.position,
        seconds: tempo ? tempo.beatsToSeconds(m.position) : (m.position * 60) / m.bpm,
        playing: m.playing,
        bpm: m.bpm,
      },
    });
    if (useListenStore.getState().countIn !== m.countIn) useListenStore.setState({ countIn: m.countIn });
  }

  dispose(): void {
    this.closeReceiver();
  }
}
