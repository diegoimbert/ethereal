import { monotonicFactory } from "ulid";
import type { GestureId } from "@/generated";

const ulid = monotonicFactory();

/**
 * A new entity id (26-char ULID). The UI generates ids for everything it creates and
 * passes them in the command (`Track::Create { id: newId(), ... }`). Monotonic, so ids
 * created in a burst sort in creation order.
 */
export function newId(): string {
  return ulid();
}

let lastGesture = 0;

/** A fresh gesture id (unique for this page; the engine only compares them for equality). */
export function nextGestureId(): GestureId {
  lastGesture = (lastGesture + 1) >>> 0 || 1;
  return lastGesture;
}
