// Standalone entry: `pnpm --filter @ethereal/ui dev` runs the UI against the mock transport.
// Hosts (apps/web, apps/desktop) have their own entries and import `App` from `@ethereal/ui`.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { isDocsPage } from "./features/docs/route";
import { KitGallery } from "./features/kit-gallery";
import "./kit/theme.css";
import { createDefaultTransport, TransportProvider } from "./transport";
import { initI18n } from "./i18n";

// MockTransport standalone; TauriTransport inside the desktop shell (see createDefaultTransport).
const transport = await createDefaultTransport();
// `?kit` shows the design-system gallery instead of the app (docs/DESIGN-SYSTEM.md).
const showKit = new URLSearchParams(location.search).has("kit");
const root = document.getElementById("root");
if (!root) throw new Error("#root element missing");

// The user's language first (a lazy chunk unless English), so nothing renders in English first.
await initI18n();
// The user docs (`/docs/…`): the web build serves them standalone; here too, for `just dev-ui`.
if (isDocsPage(location.pathname)) void import("./features/docs/mount").then((m) => m.mountDocsPage(root));
else
  createRoot(root).render(
    <StrictMode>
      <TransportProvider transport={transport}>{showKit ? <KitGallery /> : <App />}</TransportProvider>
    </StrictMode>,
  );
