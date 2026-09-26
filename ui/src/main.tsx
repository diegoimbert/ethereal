// Standalone entry: `pnpm --filter @ethereal/ui dev` runs the UI against the mock transport.
// Hosts (apps/web, apps/desktop) have their own entries and import `App` from `@ethereal/ui`.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import "./kit/theme.css";
import { createDefaultTransport, TransportProvider } from "./transport";

// MockTransport standalone; TauriTransport inside the desktop shell (see createDefaultTransport).
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
