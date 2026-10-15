import { beforeEach, describe, expect, it } from "vitest";
import { EngineHealthWatcher, SETTLE_MS, SILENCE_MS, type HealthUi } from "./watcher";

describe("EngineHealthWatcher", () => {
  let t: number;
  let log: string[];
  let shown: Map<number, string>;
  let w: EngineHealthWatcher;

  beforeEach(() => {
    t = 1000;
    log = [];
    shown = new Map();
    let id = 0;
    const ui: HealthUi = {
      warn: (m) => {
        shown.set(++id, m);
        log.push(`warn ${m}`);
        return id;
      },
      dismiss: (i) => {
        shown.delete(i);
        log.push(`dismiss ${i}`);
      },
      info: (m) => log.push(`info ${m}`),
    };
    w = new EngineHealthWatcher(ui, () => t);
  });

  const advance = (ms: number, frames = false) => {
    for (let i = 0; i < ms; i += 100) {
      t += 100;
      if (frames) w.frame();
      w.check();
    }
  };

  it("is quiet while frames flow, and before the first frame", () => {
    advance(SILENCE_MS * 2);
    w.frame();
    advance(SILENCE_MS * 2, true);
    expect(log).toEqual([]);
  });

  it("warns on a meter silence, then summarizes with the engine's reason (desktop)", () => {
    w.frame();
    advance(SILENCE_MS);
    expect([...shown.values()]).toEqual(["the engine is not responding. audio keeps playing; your actions will run when it recovers."]);
    // On recovery the queued engine events arrive with the frames.
    w.event({ type: "Unresponsive", stalled_ms: 2000, reason: "Analog Lab V is busy (opening its editor)" });
    expect([...shown.values()]).toEqual([
      "the engine is not responding: Analog Lab V is busy (opening its editor). audio keeps playing; your actions will run when it recovers.",
    ]);
    advance(300, true);
    w.event({ type: "Responsive", stalled_ms: 12400, reason: "Analog Lab V is busy (opening its editor)" });
    expect(shown.size).toBe(0);
    expect(log.at(-1)).toBe("info the engine was unresponsive for 12 s (Analog Lab V is busy (opening its editor)).");
  });

  it("summarizes without a reason when the engine never reports one", () => {
    w.frame();
    advance(SILENCE_MS + 1500);
    advance(SETTLE_MS + 100, true);
    expect(shown.size).toBe(0);
    expect(log.at(-1)).toBe("info the engine was unresponsive for 4 s.");
  });

  it("follows engine events alone (transports without steady meters)", () => {
    w.event({ type: "Unresponsive", stalled_ms: 2100, reason: null });
    w.event({ type: "Unresponsive", stalled_ms: 2600, reason: "processing a command" });
    advance(SETTLE_MS * 3);
    expect([...shown.values()]).toEqual(["the engine is not responding: processing a command. audio keeps playing; your actions will run when it recovers."]);
    w.event({ type: "Responsive", stalled_ms: 3000, reason: "processing a command" });
    expect(shown.size).toBe(0);
    expect(log.at(-1)).toBe("info the engine was unresponsive for 3 s (processing a command).");
    // A stray `Responsive` (e.g. after a reload) shows nothing.
    w.event({ type: "Responsive", stalled_ms: 3000, reason: null });
    expect(log.filter((l) => l.startsWith("info"))).toHaveLength(1);
  });

  it("does not count a hidden page's silence", () => {
    w.frame();
    w.hidden();
    advance(SILENCE_MS * 4);
    expect(log).toEqual([]);
  });
});
