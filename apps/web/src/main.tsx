// Browser host entry. OWNERSHIP: the `wasm-host` node owns apps/web/**.
// It will construct a `WasmTransport` (controller in a Worker, engine in an AudioWorklet,
// SharedArrayBuffer rings) and pass it to `<TransportProvider>`. Until then
// `createDefaultTransport()` picks the MockTransport (WasmTransport with VITE_ETHER_TRANSPORT=wasm).
import { App, createDefaultTransport, TransportProvider } from "@ethereal/ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

if (!window.crossOriginIsolated) {
  console.warn("Ethereal: page is not cross-origin isolated; SharedArrayBuffer is unavailable (check COOP/COEP headers).");
}

const transport = createDefaultTransport();
const root = document.getElementById("root");
if (!root) throw new Error("#root element missing");

createRoot(root).render(
  <StrictMode>
    <TransportProvider transport={transport}>
      <App />
    </TransportProvider>
  </StrictMode>,
);
