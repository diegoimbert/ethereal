/**
 * One tab owns the web project store (#563). Every tab of the web app runs its own engine on
 * the same OPFS folder and the same draft list (localStorage): a second tab would delete the
 * first tab's untouched draft on launch (`deleteStaleDrafts`) and both would overwrite each
 * other's saves. So the engine starts only in the tab holding the `ethereal-store` Web Lock.
 *
 * A tab that finds the lock taken shows "ethereal is open in another tab" (`OtherTabScreen`)
 * and starts nothing. "use here" asks the owner over a BroadcastChannel to hand over: the
 * owner saves, stops its engine and releases the lock, then the new tab gets it. An owner
 * that doesn't answer in `HANDOVER_TIMEOUT_MS` (a frozen tab) has the lock stolen: it is told
 * (`onLost`) and stops its engine at once.
 *
 * Without the Web Locks API (old browsers) there is no guard: the tab owns the store.
 */

export const STORE_LOCK = "ethereal-store";
/** How long "use here" waits for the owner to save and release before taking the lock. */
export const HANDOVER_TIMEOUT_MS = 8000;

/** The tab owns the store (and runs the engine) until it releases or loses the lock. */
export interface StoreOwner {
  /**
   * What to do when another tab takes over (save, stop the engine): the lock is released once
   * it settles. A request that came before this was set runs it at once.
   */
  onHandover(handover: () => Promise<void>): void;
  /** The lock was taken without a handover (stolen): stop writing at once. */
  onLost(listener: () => void): void;
  release(): void;
}

/** The parts of the browser this uses (injectable for tests). */
export interface LockEnv {
  locks: Pick<LockManager, "request"> | undefined;
  channel(name: string): Pick<BroadcastChannel, "postMessage" | "close" | "addEventListener"> | null;
}

export function browserLockEnv(): LockEnv {
  return {
    locks: typeof navigator !== "undefined" ? navigator.locks : undefined,
    channel: (name) => (typeof BroadcastChannel === "undefined" ? null : new BroadcastChannel(name)),
  };
}

const HANDOVER = "handover";

interface Held {
  release(): void;
  lost: Promise<void>;
}

/** Request the lock with `options`; `null` when `ifAvailable` found it taken. */
function hold(locks: Pick<LockManager, "request">, options: LockOptions): Promise<Held | null> {
  return new Promise((resolve, reject) => {
    let granted = false;
    let release!: () => void;
    const released = new Promise<void>((r) => (release = r));
    let markLost!: () => void;
    const lost = new Promise<void>((r) => (markLost = r));
    locks
      .request(STORE_LOCK, options, (lock) => {
        if (!lock) {
          resolve(null);
          return undefined;
        }
        granted = true;
        resolve({ release, lost });
        return released;
      })
      .then(
        () => undefined,
        (e: unknown) => {
          // Granted, then stolen by another tab: the request rejects (AbortError).
          if (granted) markLost();
          else reject(e as Error);
        },
      );
  });
}

function ownerOf(held: Held | null, env: LockEnv): StoreOwner {
  const channel = held ? env.channel(STORE_LOCK) : null;
  let handover: (() => Promise<void>) | null = null;
  let requested = false;
  let done = false;
  const lost: Array<() => void> = [];

  const release = () => {
    if (done) return;
    done = true;
    channel?.close();
    held?.release();
  };
  const handOver = () => {
    if (done || !handover) return;
    const run = handover;
    handover = null;
    void run()
      .catch((e: unknown) => console.error("ethereal: handing the store over failed", e))
      .finally(release);
  };
  channel?.addEventListener("message", (e) => {
    if ((e as MessageEvent).data?.type !== HANDOVER) return;
    requested = true;
    handOver();
  });
  void held?.lost.then(() => {
    if (done) return;
    done = true;
    channel?.close();
    for (const l of lost) l();
  });

  return {
    onHandover(h) {
      handover = h;
      if (requested) handOver();
    },
    onLost(l) {
      lost.push(l);
    },
    release,
  };
}

/** How long `claimStore` waits for a page this tab is replacing (a reload) to let go of the store. */
export const CLAIM_GRACE_MS = 1500;

/**
 * Take the store if no other tab has it; `null` when another tab still does after `graceMs`
 * (a reload's previous page may hold it a moment longer). Without Web Locks, always the owner.
 */
export async function claimStore(env: LockEnv = browserLockEnv(), graceMs = CLAIM_GRACE_MS): Promise<StoreOwner | null> {
  if (!env.locks) return ownerOf(null, env);
  let held = await hold(env.locks, { ifAvailable: true });
  if (!held && graceMs > 0) {
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(), graceMs);
    held = await hold(env.locks, { signal: abort.signal }).catch((e: unknown) => {
      if (abort.signal.aborted) return null;
      throw e;
    });
    clearTimeout(timer);
  }
  return held ? ownerOf(held, env) : null;
}

/**
 * "use here": ask the owning tab to hand over (it saves and stops), and take the lock once
 * released; after `timeoutMs` without it, take it anyway (the owner is told it lost it).
 */
export async function takeOverStore(env: LockEnv = browserLockEnv(), timeoutMs = HANDOVER_TIMEOUT_MS): Promise<StoreOwner> {
  if (!env.locks) return ownerOf(null, env);
  const ask = env.channel(STORE_LOCK);
  ask?.postMessage({ type: HANDOVER });
  ask?.close();
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), timeoutMs);
  let held: Held | null;
  try {
    held = await hold(env.locks, { signal: abort.signal });
  } catch (e) {
    if (!abort.signal.aborted) throw e;
    held = await hold(env.locks, { steal: true });
  } finally {
    clearTimeout(timer);
  }
  return ownerOf(held, env);
}
