import { describe, expect, it } from "vitest";
import type { Command, ReplyValue } from "@/generated";
import type { EngineTransport } from "./EngineTransport";
import { listProjects } from "./projectList";

/** An engine whose `Project::List` replies wait for `answer()`; each reply carries its number. */
function engine() {
  const waiting: Array<() => void> = [];
  let sent = 0;
  const transport = {
    send: (c: Command) => {
      expect(c).toEqual({ domain: "Project", command: { type: "List" } });
      const n = ++sent;
      return new Promise<ReplyValue>((resolve) => waiting.push(() => resolve({ type: "Projects", projects: [{ name: String(n) }] } as unknown as ReplyValue)));
    },
  } as unknown as EngineTransport;
  return {
    transport,
    sent: () => sent,
    answer: async () => {
      waiting.shift()!();
      // Let the queued request go out.
      for (let i = 0; i < 5; i++) await Promise.resolve();
    },
  };
}

const nameOf = (r: ReplyValue) => (r.type === "Projects" ? r.projects[0]!.name : null);

describe("listProjects", () => {
  it("keeps one request in flight and one queued, shared by every later caller", async () => {
    const e = engine();
    const first = listProjects(e.transport);
    const later = [listProjects(e.transport), listProjects(e.transport), listProjects(e.transport)];
    expect(e.sent()).toBe(1);
    await e.answer();
    expect(nameOf(await first)).toBe("1");
    // The callers that asked while the first was in flight get a fresh scan, one for all.
    expect(e.sent()).toBe(2);
    await e.answer();
    for (const r of await Promise.all(later)) expect(nameOf(r)).toBe("2");
    // Idle again: the next call goes straight out.
    void listProjects(e.transport);
    expect(e.sent()).toBe(3);
  });

  it("a failed request doesn't stall the queue", async () => {
    let sent = 0;
    const transport = {
      send: () => (++sent === 1 ? Promise.reject(new Error("busy")) : Promise.resolve({ type: "Projects", projects: [] } as unknown as ReplyValue)),
    } as unknown as EngineTransport;
    const first = listProjects(transport);
    const second = listProjects(transport);
    await expect(first).rejects.toThrow("busy");
    expect((await second).type).toBe("Projects");
    expect(sent).toBe(2);
  });
});
