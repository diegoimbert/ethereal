// Browser host entry.
//
// The UI talks to the wasm engine through `WasmTransport`: the real `EtherController` runs
// in a Web Worker, the engine in an AudioWorklet, connected by SharedArrayBuffer rings
// (./engine/endpoint.ts). The standalone UI against `MockTransport` is `just dev-ui`.
import { App, playheadStore, TransportProvider, useProjectStore, WasmTransport } from "@ethereal/ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createWebEndpoint } from "./engine/endpoint";

const endpoint = createWebEndpoint();
// Debug / e2e handle on the live engine (AudioContext, worklet node, workers).
(window as unknown as { __etherEngine?: unknown }).__etherEngine = endpoint;
const transport = new WasmTransport({ endpoint });
// Read-only e2e/debug view of the UI mirror: the project document, history, dirty flag
// and the latest meter readings (apps/web/e2e reads these to check state across reloads).
(window as unknown as { __ether?: unknown }).__ether = {
  state: () => useProjectStore.getState(),
  meter: (track: string) => playheadStore.getMeter(track),
};

const root = document.getElementById("root");
if (!root) throw new Error("#root element missing");

createRoot(root).render(
  <StrictMode>
    <TransportProvider transport={transport}>
      <App />
    </TransportProvider>
  </StrictMode>,
);
