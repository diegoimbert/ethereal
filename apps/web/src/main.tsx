// Browser host entry: a tiny gate. Everything loads ./app.tsx (the editor, or the mobile UI
// on phones and touch tablets (./mobile.ts), and the wasm engine). The user docs (`/docs/…`,
// #665) are their own small bundle too.
import { isDocsPage } from "@/features/docs/route";

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("#root element missing");

if (isDocsPage(location.pathname)) void import("@/features/docs/mount").then((m) => m.mountDocsPage(rootElement));
else void import("./app");
