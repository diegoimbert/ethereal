/** Shared setup for the roadmap v2 mock tests (one `*.test.ts` per feature node). */
import { afterEach, beforeEach } from "vitest";
import type { Event, Project } from "@/generated";
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
