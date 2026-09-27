// Peers' live arranger pointers (`CollabEvent::Pointer`), with their recent samples for
// interpolation. `sites` changes only when a peer's pointer appears or goes away (React
// renders one element per peer); the positions are read per frame from `trails`.
import { create } from "zustand";
import type { ArrangerPointer, SiteId } from "@/generated";
import { PointerTrail } from "./interp";

export interface PointerState {
  /** Peers with a pointer, in arrival order. */
  sites: ReadonlyArray<SiteId>;
  trails: ReadonlyMap<SiteId, PointerTrail>;
  /** Bumped on every sample (the overlay restarts its animation loop). */
  tick: number;
  onPointer(site: SiteId, pointer: ArrangerPointer | null, now?: number): void;
  clear(): void;
}

export const usePointerStore = create<PointerState>()((set, get) => ({
  sites: [],
  trails: new Map(),
  tick: 0,
  onPointer: (site, pointer, now = performance.now()) => {
    const { sites, trails, tick } = get();
    if (!pointer) {
      if (!trails.has(site)) return;
      const next = new Map(trails);
      next.delete(site);
      set({ trails: next, sites: sites.filter((s) => s !== site), tick: tick + 1 });
      return;
    }
    let trail = trails.get(site);
    if (!trail) {
      trail = new PointerTrail();
      trail.push(pointer, now);
      set({ trails: new Map(trails).set(site, trail), sites: [...sites, site], tick: tick + 1 });
      return;
    }
    trail.push(pointer, now);
    set({ tick: tick + 1 });
  },
  clear: () => set({ sites: [], trails: new Map(), tick: 0 }),
}));
