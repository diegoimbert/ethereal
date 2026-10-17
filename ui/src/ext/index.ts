/**
 * UI extension points (docs/EXTENSIONS.md): the places where code outside this repository
 * (an extension, e.g. the official build's private features) adds to the app without
 * changing its files. An extension calls `contribute(point, value)` before the app renders
 * (from its entry module, ahead of the app's own); the app reads the contributions where the
 * point is used. Nothing is contributed in this build: every point then shows the app as is.
 *
 *   contribute("palette.commands", (ctx) => [...]);
 *   contribute("shell.root", { name: "my feature", Component: MyRoot });
 *
 * The points and their value types are `ExtensionPoints` (`./points.ts`), an interface an
 * extension may augment for points of its own.
 */
import { useSyncExternalStore } from "react";
import type { ExtensionPoints } from "./points";

export type * from "./points";

type Point = keyof ExtensionPoints;

interface Entry<K extends Point> {
  value: ExtensionPoints[K];
  order: number;
  seq: number;
}

const entries = new Map<Point, Entry<Point>[]>();
const cache = new Map<Point, ReadonlyArray<unknown>>();
const listeners = new Set<() => void>();
let seq = 0;

const EMPTY: ReadonlyArray<never> = Object.freeze([]);

/**
 * Add `value` to `point`. `order` sorts the contributions of a point (ascending, then in
 * contribution order); the app's own items sit at 0 where a point mixes both. Returns a
 * function that takes it back (tests, hot reload).
 */
export function contribute<K extends Point>(point: K, value: ExtensionPoints[K], order = 0): () => void {
  const entry: Entry<K> = { value, order, seq: seq++ };
  const list = (entries.get(point) ?? []) as Entry<K>[];
  list.push(entry);
  list.sort((a, b) => a.order - b.order || a.seq - b.seq);
  entries.set(point, list as Entry<Point>[]);
  changed(point);
  return () => {
    const now = entries.get(point);
    const i = now?.indexOf(entry as Entry<Point>) ?? -1;
    if (now && i >= 0) {
      now.splice(i, 1);
      changed(point);
    }
  };
}

function changed(point: Point): void {
  cache.delete(point);
  for (const l of listeners) l();
}

/** The contributions to `point`, sorted (a stable array until the point changes). */
export function contributions<K extends Point>(point: K): ReadonlyArray<ExtensionPoints[K]> {
  let list = cache.get(point) as ReadonlyArray<ExtensionPoints[K]> | undefined;
  if (!list) {
    const e = entries.get(point);
    list = e?.length ? Object.freeze(e.map((x) => x.value as ExtensionPoints[K])) : EMPTY;
    cache.set(point, list);
  }
  return list;
}

/** `contributions(point)` for a component: re-renders when the point changes. */
export function useContributions<K extends Point>(point: K): ReadonlyArray<ExtensionPoints[K]> {
  return useSyncExternalStore(subscribe, () => contributions(point));
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** Whether anything is contributed to `point` (a feature the app shows only when extended). */
export const hasContributions = (point: Point): boolean => contributions(point).length > 0;

/** Call every function contributed to `point` with `args` (events: analytics, crashes, …). */
export function emit<K extends Point>(point: K, ...args: ExtensionPoints[K] extends (...a: infer A) => unknown ? A : never): void {
  for (const f of contributions(point)) {
    try {
      (f as (...a: unknown[]) => unknown)(...args);
    } catch (e) {
      console.warn(`[ethereal] extension point ${point} failed:`, e);
    }
  }
}

/** Tests: forget every contribution. */
export function resetExtensionsForTests(): void {
  entries.clear();
  cache.clear();
  for (const l of listeners) l();
}
