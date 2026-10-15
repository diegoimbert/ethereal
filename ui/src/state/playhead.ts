/**
 * High-rate engine data (playhead ~60 Hz, meters ~30 Hz), kept OUT of the project store and
 * out of React state so that updates only re-render the few components that read them.
 *
 * - Canvas/rAF code: read `playheadStore.getPlayhead()` / `getMeter(id)` directly, or
 *   `subscribe…` imperatively.
 * - React: `usePlayhead()`, `usePlayheadPosition()`, `useTrackMeter(trackId)`,
 *   `useCpuLoad()` (each subscribes via `useSyncExternalStore` to just that value; meters
 *   notify per track).
 *
 * `TransportProvider` feeds it from `EngineTransport.subscribePlayhead/subscribeMeters`.
 */

import { useCallback, useSyncExternalStore } from "react";
import type { MeterFrame, PlayheadFrame, TrackId, TrackMeter } from "@/generated";

type Listener = () => void;

const sameMeter = (a: TrackMeter, b: TrackMeter): boolean =>
  a.peak[0] === b.peak[0] && a.peak[1] === b.peak[1] && a.rms[0] === b.rms[0] && a.rms[1] === b.rms[1] && a.clipped === b.clipped;

class HighRateStore {
  private playhead: PlayheadFrame | null = null;
  private overridden = false;
  private meters = new Map<TrackId, TrackMeter>();
  private cpuLoad = 0;
  private playheadListeners = new Set<Listener>();
  private meterListeners = new Map<TrackId, Set<Listener>>();
  private cpuListeners = new Set<Listener>();

  // ── Playhead ──
  getPlayhead = (): PlayheadFrame | null => this.playhead;

  /** An engine frame (ignored while an override is active). */
  setPlayhead(frame: PlayheadFrame): void {
    if (this.overridden) return;
    this.playhead = frame;
    for (const l of this.playheadListeners) l();
  }

  /**
   * Listen on a peer (docs/COLLAB.md §9.4): show `frame` (what is heard from the host)
   * and ignore the local engine's frames until `setOverride(null)`.
   */
  setOverride(frame: PlayheadFrame | null): void {
    this.overridden = frame !== null;
    if (!frame) return;
    this.playhead = frame;
    for (const l of this.playheadListeners) l();
  }

  subscribePlayhead = (listener: Listener): (() => void) => {
    this.playheadListeners.add(listener);
    return () => this.playheadListeners.delete(listener);
  };

  // ── Meters ──
  getMeter(track: TrackId): TrackMeter | null {
    return this.meters.get(track) ?? null;
  }

  getCpuLoad = (): number => this.cpuLoad;

  /** Tracks missing from `frame` keep their last reading (UIs decay them visually). */
  setMeters(frame: MeterFrame): void {
    for (const m of frame.tracks) {
      // The engine sends every track each tick: a silent or steady track keeps its snapshot
      // (same object) and re-renders nothing.
      const prev = this.meters.get(m.track);
      if (prev && sameMeter(prev, m)) continue;
      this.meters.set(m.track, m);
      const ls = this.meterListeners.get(m.track);
      if (ls) for (const l of ls) l();
    }
    if (frame.cpu_load !== this.cpuLoad) {
      this.cpuLoad = frame.cpu_load;
      for (const l of this.cpuListeners) l();
    }
  }

  subscribeMeter(track: TrackId, listener: Listener): () => void {
    let ls = this.meterListeners.get(track);
    if (!ls) this.meterListeners.set(track, (ls = new Set()));
    ls.add(listener);
    return () => {
      ls.delete(listener);
      if (ls.size === 0) this.meterListeners.delete(track);
    };
  }

  subscribeCpu = (listener: Listener): (() => void) => {
    this.cpuListeners.add(listener);
    return () => this.cpuListeners.delete(listener);
  };

  /** Clear all values (listeners are kept) and notify. */
  reset(): void {
    this.playhead = null;
    this.overridden = false;
    this.cpuLoad = 0;
    const tracks = [...this.meters.keys()];
    this.meters.clear();
    for (const l of this.playheadListeners) l();
    for (const t of tracks) for (const l of this.meterListeners.get(t) ?? []) l();
    for (const l of this.cpuListeners) l();
  }
}

/** The app-wide high-rate store (one engine per app). */
export const playheadStore = new HighRateStore();

/** Latest playhead frame (`null` before the first one). Re-renders at playhead rate. */
export function usePlayhead(): PlayheadFrame | null {
  return useSyncExternalStore(playheadStore.subscribePlayhead, playheadStore.getPlayhead, playheadStore.getPlayhead);
}

/** Playhead position in beats (re-renders only when it changes). */
export function usePlayheadPosition(): number {
  const get = () => playheadStore.getPlayhead()?.transport.position ?? 0;
  return useSyncExternalStore(playheadStore.subscribePlayhead, get, get);
}

/** Latest meter reading of one track (`null` until the first one). */
export function useTrackMeter(track: TrackId): TrackMeter | null {
  const subscribe = useCallback((l: Listener) => playheadStore.subscribeMeter(track, l), [track]);
  const get = useCallback(() => playheadStore.getMeter(track), [track]);
  return useSyncExternalStore(subscribe, get, get);
}

/** Engine DSP load 0..=1. */
export function useCpuLoad(): number {
  return useSyncExternalStore(playheadStore.subscribeCpu, playheadStore.getCpuLoad, playheadStore.getCpuLoad);
}
