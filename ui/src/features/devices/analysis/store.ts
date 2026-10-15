/**
 * UI side of the analysis channel (CONTRACTS.md §12.4.3): one store per transport that
 *
 * - refcounts the widgets watching a device and sends `Analysis::Watch` on the first one and
 *   `Analysis::Unwatch` after the last one (the controller refcounts per connection too, so
 *   one watch per device per UI is enough);
 * - releases every watch while the page is hidden (`document.visibilityState`) and takes
 *   them again when it shows, so a background tab costs the engine nothing;
 * - keeps the latest `Event::Analysis` frame per device and kind (spectrum: per stage) and
 *   notifies only that device's subscribers (≤ 30 Hz per device).
 */

import type { AnalysisData, DeviceId, Event } from "@/generated";
import { cmd, type EngineTransport, type Unsubscribe } from "@/transport";

const EMPTY: ReadonlyArray<AnalysisData> = Object.freeze([]);

/** Latest-frame slot of `data` (one per kind, spectrum per stage). */
function slotOf(data: AnalysisData): string {
  return data.type === "Spectrum" ? `Spectrum:${data.stage}` : data.type;
}

function pageVisible(): boolean {
  return typeof document === "undefined" || document.visibilityState !== "hidden";
}

export class AnalysisStore {
  /** Device → widgets watching it. */
  private readonly counts = new Map<DeviceId, number>();
  /** Devices the engine was told to watch (a subset of `counts` while visible). */
  private readonly engineWatched = new Set<DeviceId>();
  private readonly latest = new Map<DeviceId, ReadonlyArray<AnalysisData>>();
  private readonly listeners = new Map<DeviceId, Set<() => void>>();
  private offEvent: Unsubscribe | null = null;
  private visible = pageVisible();
  private readonly onVisibility = () => this.setVisible(pageVisible());

  constructor(private readonly transport: EngineTransport) {}

  /** Watch `device` until the returned function is called (refcounted). */
  watch(device: DeviceId): () => void {
    if (this.counts.size === 0) this.attach();
    this.counts.set(device, (this.counts.get(device) ?? 0) + 1);
    this.sync(device);
    let done = false;
    return () => {
      if (done) return;
      done = true;
      const n = (this.counts.get(device) ?? 1) - 1;
      if (n > 0) {
        this.counts.set(device, n);
        return;
      }
      this.counts.delete(device);
      this.sync(device);
      // Stale frames of an unwatched device must not show when it is watched again.
      if (this.latest.delete(device)) this.notify(device);
      if (this.counts.size === 0) this.detach();
    };
  }

  /** The latest frames of `device` (stable reference until a new frame arrives). */
  frames(device: DeviceId): ReadonlyArray<AnalysisData> {
    return this.latest.get(device) ?? EMPTY;
  }

  subscribe(device: DeviceId, listener: () => void): Unsubscribe {
    let set = this.listeners.get(device);
    if (!set) this.listeners.set(device, (set = new Set()));
    set.add(listener);
    return () => {
      set.delete(listener);
      if (set.size === 0) this.listeners.delete(device);
    };
  }

  /** Watch count of `device` (tests, diagnostics). */
  watchers(device: DeviceId): number {
    return this.counts.get(device) ?? 0;
  }

  /** Whether the engine is currently asked for `device`'s frames. */
  engineWatching(device: DeviceId): boolean {
    return this.engineWatched.has(device);
  }

  /** Page visibility changed (also driven by `visibilitychange`). */
  setVisible(visible: boolean): void {
    if (visible === this.visible) return;
    this.visible = visible;
    for (const device of new Set([...this.counts.keys(), ...this.engineWatched])) this.sync(device);
  }

  /** Feed one event (the transport listener). */
  handle(event: Event): void {
    if (event.type !== "Analysis" || event.event.type !== "Frame") return;
    const { device, data } = event.event;
    if (!this.counts.has(device)) return;
    const prev = this.latest.get(device) ?? EMPTY;
    const slot = slotOf(data);
    const i = prev.findIndex((d) => slotOf(d) === slot);
    const next = i < 0 ? [...prev, data] : prev.map((d, j) => (j === i ? data : d));
    this.latest.set(device, next);
    this.notify(device);
  }

  private sync(device: DeviceId): void {
    const want = this.visible && this.counts.has(device);
    if (want === this.engineWatched.has(device)) return;
    if (want) this.engineWatched.add(device);
    else this.engineWatched.delete(device);
    const command = want ? cmd("Analysis", { type: "Watch", device }) : cmd("Analysis", { type: "Unwatch", device });
    // Watches are best-effort: a host without the channel (or a dropped connection, whose
    // watches the server releases) just shows no frames.
    this.transport.send(command).catch(() => {});
  }

  private notify(device: DeviceId): void {
    for (const l of this.listeners.get(device) ?? []) l();
  }

  private attach(): void {
    this.offEvent = this.transport.onEvent((e) => this.handle(e));
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", this.onVisibility);
      this.visible = pageVisible();
    }
  }

  private detach(): void {
    this.offEvent?.();
    this.offEvent = null;
    if (typeof document !== "undefined") document.removeEventListener("visibilitychange", this.onVisibility);
  }
}

const stores = new WeakMap<EngineTransport, AnalysisStore>();

/** The analysis store of `transport` (one per connection). */
export function analysisStore(transport: EngineTransport): AnalysisStore {
  let s = stores.get(transport);
  if (!s) stores.set(transport, (s = new AnalysisStore(transport)));
  return s;
}
