// Controller Worker: runs `ether-controller` (wasm instance #1). Receives JSON
// `ClientMessage`s from the UI, talks to the AudioWorklet over the SAB rings (inside Rust),
// persists projects in OPFS through `SyncFs`, and ticks at ~60 Hz to forward playhead and
// meters.
import * as wasm from "@ether-wasm/ether_wasm.js";
import { initSync, WasmController } from "@ether-wasm/ether_wasm.js";
import type { FromController, ShareProbeMethod, ToController } from "./protocol";
import { SyncFs } from "./syncFs";

interface Scope {
  onmessage: ((e: MessageEvent<ToController>) => void) | null;
  postMessage(msg: FromController): void;
}
const scope = self as unknown as Scope;

/**
 * The exports of crates/ether-wasm/src/share.rs (typed here: ether-wasm.d.ts belongs to the
 * wasm host; these two are the share port's, node `p2p-transport`).
 */
interface ShareProbe {
  free(): void;
}
interface ShareExports {
  install_share_port(port: MessagePort): void;
  ShareProbe: new () => ShareProbe;
}
const share = wasm as unknown as ShareExports;
let probe: ShareProbe | null = null;

function callProbe(method: ShareProbeMethod, args: unknown[]): unknown {
  probe ??= new share.ShareProbe();
  const f = (probe as unknown as Record<string, (...a: unknown[]) => unknown>)[method];
  if (typeof f !== "function") throw new Error(`no ShareProbe.${method}`);
  return f.apply(probe, args);
}

const TICK_MS = 16;
let controller: WasmController | null = null;
let dead = false;

function post(json: string): void {
  if (json !== "[]") scope.postMessage({ type: "server", json });
}

function fatal(e: unknown): void {
  if (dead) return;
  dead = true;
  const message = e instanceof Error ? e.message : String(e);
  scope.postMessage({ type: "fatal", message });
}

/** A Rust panic aborts the wasm instance: every later call would fail too. */
function guarded(f: () => void): void {
  if (dead) return;
  try {
    f();
  } catch (e) {
    fatal(e);
  }
}

scope.onmessage = (e) => {
  const msg = e.data;
  switch (msg.type) {
    case "init":
      guarded(() => {
        initSync({ module: msg.module });
        const fs = new SyncFs(msg.fsBuffer, msg.fsPort);
        const ctl = new WasmController(BigInt(msg.seed), msg.sampleRate, msg.control, msg.reports, fs);
        controller = ctl;
        share.install_share_port(msg.sharePort);
        setInterval(() => guarded(() => post(ctl.tick(Date.now()))), TICK_MS);
        scope.postMessage({ type: "ready" });
      });
      break;
    case "client":
      guarded(() => {
        if (!controller) throw new Error("controller not initialized");
        post(controller.handle(msg.json));
      });
      break;
    case "share-probe": {
      // Not `guarded`: a probe error is the caller's, not the engine's.
      if (dead) break;
      try {
        scope.postMessage({ type: "share-probe", id: msg.id, result: callProbe(msg.method, msg.args) });
      } catch (e) {
        scope.postMessage({ type: "share-probe", id: msg.id, error: e instanceof Error ? e.message : String(e) });
      }
      break;
    }
  }
};
