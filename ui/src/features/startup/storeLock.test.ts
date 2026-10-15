import { afterEach, describe, expect, it, vi } from "vitest";
import { claimStore, takeOverStore, type LockEnv } from "./storeLock";

/** An in-memory Web Locks manager (exclusive locks, `ifAvailable`, `steal`, `signal`) and BroadcastChannel, shared by "tabs". */
function browser() {
  type Waiter = { grant(): void; abort?(): void };
  let holder: { end(): void; steal(): void } | null = null;
  const queue: Waiter[] = [];
  const next = () => {
    if (!holder) queue.shift()?.grant();
  };
  const locks = {
    request(_name: string, options: LockOptions, cb: (lock: Lock | null) => unknown): Promise<unknown> {
      return new Promise((resolve, reject) => {
        const run = () => {
          let stolen = false;
          holder = {
            end: () => undefined,
            steal: () => {
              stolen = true;
              reject(new DOMException("stolen", "AbortError"));
            },
          };
          void Promise.resolve(cb({ name: "ethereal-store", mode: "exclusive" } as Lock)).then((v) => {
            if (stolen) return;
            holder = null;
            resolve(v);
            next();
          });
        };
        if (options.steal) {
          holder?.steal();
          holder = null;
          run();
        } else if (!holder) run();
        else if (options.ifAvailable) void Promise.resolve(cb(null)).then(resolve);
        else {
          const w: Waiter = { grant: run };
          queue.push(w);
          options.signal?.addEventListener("abort", () => {
            queue.splice(queue.indexOf(w), 1);
            reject(new DOMException("aborted", "AbortError"));
          });
        }
      });
    },
  } as unknown as LockManager;
  const channels = new Set<{ listeners: ((e: MessageEvent) => void)[]; closed: boolean }>();
  const tab = (): LockEnv => ({
    locks,
    channel: () => {
      const me = { listeners: [] as ((e: MessageEvent) => void)[], closed: false };
      channels.add(me);
      return {
        postMessage(data: unknown) {
          for (const c of channels) if (c !== me && !c.closed) for (const l of c.listeners) setTimeout(() => l({ data } as MessageEvent));
        },
        close() {
          me.closed = true;
          channels.delete(me);
        },
        addEventListener(_type: string, l: (e: MessageEvent) => void) {
          me.listeners.push(l);
        },
      } as unknown as BroadcastChannel;
    },
  });
  return { tab };
}

afterEach(() => vi.useRealTimers());

describe("storeLock (#563)", () => {
  it("the first tab owns the store; a second one is refused until the first releases", async () => {
    const b = browser();
    const first = await claimStore(b.tab());
    expect(first).not.toBeNull();
    expect(await claimStore(b.tab(), 0)).toBeNull();
    // The grace (a reload's previous page letting go) runs out: still refused.
    expect(await claimStore(b.tab(), 30)).toBeNull();
    first!.release();
    expect(await claimStore(b.tab(), 100)).not.toBeNull();
  });

  it("use here: the owner hands over (saves) before the new tab gets the store", async () => {
    const b = browser();
    const first = (await claimStore(b.tab()))!;
    const order: string[] = [];
    first.onHandover(async () => {
      await new Promise((r) => setTimeout(r, 20));
      order.push("saved");
    });
    const lost = vi.fn();
    first.onLost(lost);
    await takeOverStore(b.tab());
    order.push("taken");
    expect(order).toEqual(["saved", "taken"]);
    expect(lost).not.toHaveBeenCalled();
  });

  it("a handover asked for before the owner is ready runs once it is", async () => {
    const b = browser();
    const first = (await claimStore(b.tab()))!;
    const taking = takeOverStore(b.tab());
    await new Promise((r) => setTimeout(r, 10));
    const handover = vi.fn(() => Promise.resolve());
    first.onHandover(handover);
    await taking;
    expect(handover).toHaveBeenCalledOnce();
  });

  it("an owner that never answers has the store taken after the timeout and is told", async () => {
    const b = browser();
    const first = (await claimStore(b.tab()))!;
    first.onHandover(() => new Promise(() => undefined));
    const lost = vi.fn();
    first.onLost(lost);
    const second = await takeOverStore(b.tab(), 30);
    expect(second).toBeTruthy();
    await vi.waitFor(() => expect(lost).toHaveBeenCalledOnce());
    // The new owner holds it: a third tab is refused.
    expect(await claimStore(b.tab(), 0)).toBeNull();
  });

  it("without Web Locks there is no guard", async () => {
    const env: LockEnv = { locks: undefined, channel: () => null };
    expect(await claimStore(env)).not.toBeNull();
    expect(await claimStore(env)).not.toBeNull();
  });
});
