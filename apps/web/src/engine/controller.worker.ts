// Controller Worker: runs `ether-controller` (wasm instance #1). Receives JSON
// `ClientMessage`s from the UI, talks to the AudioWorklet over the SAB rings (inside Rust),
// persists projects in OPFS through `SyncFs`, and ticks at ~60 Hz to forward playhead and
// meters.
import { initSync, WasmController } from "@ether-wasm/ether_wasm.js";
import type { FromController, ToController } from "./protocol";
import { SyncFs } from "./syncFs";

interface Scope {
  onmessage: ((e: MessageEvent<ToController>) => void) | null;
  postMessage(msg: FromController): void;
}
const scope = self as unknown as Scope;

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
        const ctl = new WasmController(BigInt(msg.seed), msg.mode, msg.control, msg.reports, fs);
        controller = ctl;
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
  }
};
