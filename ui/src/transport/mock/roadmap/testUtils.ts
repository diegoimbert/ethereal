/** Shared setup for the roadmap v2 mock tests (one `*.test.ts` per feature node). */
import { afterEach, beforeEach, expect } from "vitest";
import type { Command, Event, Project, TrackKind } from "@/generated";
import { cmd } from "../../cmd";
import { MockTransport } from "../MockTransport";

export interface MockFixture {
  mock: MockTransport;
  events: Event[];
}

/** A connected manual-timer MockTransport (seed 7) per test, with the events it emitted. */
export function useMock(): MockFixture {
  const f = { events: [] as Event[] } as MockFixture;
  beforeEach(async () => {
    f.mock = new MockTransport({ timers: "manual", seed: 7 });
    f.events = [];
    f.mock.onEvent((e) => f.events.push(e));
    await f.mock.connect();
  });
  afterEach(() => f.mock.dispose());
  return f;
}

export const project = (f: MockFixture): Project => f.mock.snapshot();
export const trackNamed = (f: MockFixture, name: string) => Object.values(project(f).tracks).find((t) => t.name === name)!;
export const undo = (f: MockFixture) => f.mock.send(cmd("Edit", { type: "Undo" }));

let seq = 0;
/** A fresh, valid entity id for tests. */
export const testId = (): string => `01K${String(++seq).padStart(23, "0")}`;

/** Create a track of `kind`; returns its id. */
export async function createTrack(f: MockFixture, kind: TrackKind): Promise<string> {
  const id = testId();
  await f.mock.send(cmd("Track", { type: "Create", id, kind, name: null, color: null, parent: null, before: null }));
  return id;
}

/**
 * v0.3 (contracts-4): `command` fails `Unsupported` and leaves the document unchanged (the
 * pin each v0.3 node replaces with real tests, like `assert_unsupported` in
 * `crates/ether-controller/tests/roadmap_v4.rs`).
 */
export async function expectUnsupported(f: MockFixture, command: Command): Promise<void> {
  const before = project(f);
  await expect(f.mock.send(command)).rejects.toMatchObject({ code: "Unsupported" });
  expect(project(f)).toEqual(before);
}
