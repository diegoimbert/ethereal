// Browser host entry.
//
// The UI talks to the wasm engine through `WasmTransport`: the real `EtherController` runs
// in a Web Worker, the engine in an AudioWorklet, connected by SharedArrayBuffer rings
// (./engine/endpoint.ts). The standalone UI against `MockTransport` is `just dev-ui`; `?mock`
// runs this build against it too (e2e of mock-only fixtures, e.g. the 10,000-param plugin).
//
// Invite links (`/join/<room>#<key>`, docs/SHARING.md §5) first show the join landing
// (`ui/src/features/share/join`), which needs no engine: the engine boots only for
// "Continue in browser" (or straight away when the browser choice is remembered, and on
// phones). The key is removed from the address bar once the engine has the invite.
import { App, MockTransport, playheadStore, TransportProvider, useProjectStore, WasmTransport, type EngineTransport } from "@ethereal/ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { openInvite } from "@/features/share/join/store";
import { JoinLanding } from "@/features/share/join/JoinLanding";
import { isMobile, joinRoute, prefersBrowser } from "@/features/share/join/landing";
import { createWebEndpoint } from "./engine/endpoint";

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("#root element missing");
const root = createRoot(rootElement);

/** Drop the invite (and its key) from the address bar and history entry. */
const leaveJoinRoute = () => history.replaceState(history.state, "", "/");

async function boot(invite: string | null): Promise<void> {
  if (invite) openInvite(invite, leaveJoinRoute);
  else if (location.pathname.startsWith("/join/")) leaveJoinRoute();

  let transport: EngineTransport;
  if (new URLSearchParams(window.location.search).has("mock")) {
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
  // e2e only: the join flow against the UI's share mock (./join/mockShare.ts).
  if ((window as { __etherMockShare?: unknown }).__etherMockShare === true) {
    const { withMockShare } = await import("./join/mockShare");
    transport = withMockShare(transport);
  }
  // Read-only e2e/debug view of the UI mirror: the project document, history, dirty flag
  // and the latest meter readings (apps/web/e2e reads these to check state across reloads).
  (window as unknown as { __ether?: unknown }).__ether = {
    state: () => useProjectStore.getState(),
    meter: (track: string) => playheadStore.getMeter(track),
  };

  root.render(
    <StrictMode>
      <TransportProvider transport={transport}>
        <App />
      </TransportProvider>
    </StrictMode>,
  );
}

const route = joinRoute(location);
if (!route) {
  void boot(null);
} else if (!route.problem && (prefersBrowser() || isMobile())) {
  void boot(route.link);
} else {
  root.render(
    <StrictMode>
      <JoinLanding route={route} onContinue={() => void boot(route.link)} onOpenApp={() => void boot(null)} />
    </StrictMode>,
  );
}
