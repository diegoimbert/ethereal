// Standalone entry: `pnpm --filter @ethereal/ui dev` runs the UI against the mock transport.
// Hosts (apps/web, apps/desktop) have their own entries and import `App` from `@ethereal/ui`.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import "./kit/theme.css";

// TODO(foundation/transport): once `ui/src/transport/` exists, render
//   <TransportProvider transport={new MockTransport()}><App /></TransportProvider>
// (import both from "@/transport"). Hosts will pass their own transport instead.
const root = document.getElementById("root");
if (!root) throw new Error("#root element missing");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
