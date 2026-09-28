// Peers' playheads (docs/COLLAB.md §12.3): each peer's `PresenceState::transport` is a
// sample (position, playing, loop) taken on its clock. Clocks are not synchronized, so a
// sample is extrapolated from its LOCAL arrival time, over the replicated tempo map, and
// wrapped into the peer's loop. Pure, plus a small arrival tracker.
import type { Beats, PeerTransport, SiteId } from "@/generated";
import type { TempoMap } from "@/timeline";

/** `PEER_TRANSPORT_REFRESH_MS`: a playing peer re-sends at least this often. */
export const PEER_TRANSPORT_REFRESH_MS = 1000;
/** Past this long without a new sample, hold the last extrapolated position (don't run away). */
export const HOLD_AFTER_MS = 2 * PEER_TRANSPORT_REFRESH_MS;

export interface PeerSample {
  transport: PeerTransport;
  /** Local arrival time (`performance.now()` ms). */
  arrivedAt: number;
}

/** Where a peer's playhead is at local time `now` (ms, same clock as `arrivedAt`). */
export function extrapolate(sample: PeerSample, now: number, tempo: TempoMap): Beats {
  const t = sample.transport;
  if (!t.playing) return t.position;
  const elapsed = Math.min(Math.max(0, now - sample.arrivedAt), HOLD_AFTER_MS) / 1000;
  if (elapsed === 0) return t.position;
  const start = tempo.beatsToSeconds(t.position);
  let seconds = start + elapsed;
  const loop = t.loop_region;
  if (loop && loop.end > loop.start && t.position >= loop.start && t.position < loop.end) {
    const ls = tempo.beatsToSeconds(loop.start);
    const le = tempo.beatsToSeconds(loop.end);
    const len = le - ls;
    if (len > 0 && seconds >= le) seconds = ls + ((seconds - ls) % len);
  }
  return tempo.secondsToBeats(seconds);
}

/**
 * Tracks sample arrivals per site: a new `sent_at_ms` (or a change of anything else) is a
 * new sample, arriving now.
 */
export class SampleTracker {
  private readonly samples = new Map<SiteId, PeerSample>();

  /** Record the current transports; returns the samples of the sites that have one. */
  update(peers: ReadonlyArray<{ site: SiteId; state: { transport?: PeerTransport | null } }>, now: number): Map<SiteId, PeerSample> {
    const seen = new Set<SiteId>();
    for (const p of peers) {
      const t = p.state.transport;
      if (!t) continue;
      seen.add(p.site);
      const prev = this.samples.get(p.site);
      if (!prev || !sameSample(prev.transport, t)) this.samples.set(p.site, { transport: t, arrivedAt: now });
    }
    for (const site of [...this.samples.keys()]) if (!seen.has(site)) this.samples.delete(site);
    return this.samples;
  }

  get(site: SiteId): PeerSample | undefined {
    return this.samples.get(site);
  }
}

function sameSample(a: PeerTransport, b: PeerTransport): boolean {
  return (
    a.sent_at_ms === b.sent_at_ms &&
    a.position === b.position &&
    a.playing === b.playing &&
    a.loop_region?.start === b.loop_region?.start &&
    a.loop_region?.end === b.loop_region?.end
  );
}
