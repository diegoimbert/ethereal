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
import type { ClipId, MeterFrame, PlayheadFrame, SessionPlayback, TrackId, TrackMeter } from "@/generated";

type Listener = () => void;

class HighRateStore {
  private playhead: PlayheadFrame | null = null;
  private meters = new Map<TrackId, TrackMeter>();
  private cpuLoad = 0;
  private playheadListeners = new Set<Listener>();
  private meterListeners = new Map<TrackId, Set<Listener>>();
  private cpuListeners = new Set<Listener>();

  // ── Playhead ──
  getPlayhead = (): PlayheadFrame | null => this.playhead;

  setPlayhead(frame: PlayheadFrame): void {
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

/** Session playback of one clip (for its progress indicator), or `null` if not playing. */
export function useSessionPlayback(clip: ClipId): SessionPlayback | null {
  const get = useCallback(
    () => playheadStore.getPlayhead()?.session.find((s) => s.clip === clip) ?? null,
    [clip],
  );
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
