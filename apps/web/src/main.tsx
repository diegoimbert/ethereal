// Browser host entry.
//
// The UI talks to the wasm engine through `WasmTransport`: the real `EtherController` runs
// in a Web Worker, the engine in an AudioWorklet, connected by SharedArrayBuffer rings
// (./engine/endpoint.ts). The standalone UI against `MockTransport` is `just dev-ui`; `?mock`
// runs this build against it too (e2e of mock-only fixtures, e.g. the 10,000-param plugin).
import { App, MockTransport, playheadStore, TransportProvider, useProjectStore, WasmTransport, type EngineTransport } from "@ethereal/ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createWebEndpoint } from "./engine/endpoint";

const mock = new URLSearchParams(window.location.search).has("mock");
let transport: EngineTransport;
if (mock) {
  const m = new MockTransport();
  // e2e handle: send commands the UI has no button for (e.g. insert a mock plugin).
  (window as unknown as { __etherMock?: unknown }).__etherMock = m;
  transport = m;
} else {
  const endpoint = createWebEndpoint();
  // Debug / e2e handle on the live engine (AudioContext, worklet node, workers).
  (window as unknown as { __etherEngine?: unknown }).__etherEngine = endpoint;
  transport = new WasmTransport({ endpoint });
}
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
