// Browser host entry: a tiny gate. Everything loads ./app.tsx (the editor, or the mobile UI
// on phones and touch tablets (./mobile.ts), and the wasm engine). The user docs (`/docs/…`,
// #665) are their own small bundle too, and so is any page an extension serves (`@/ext`
// "web.route", registered before this module runs; docs/EXTENSIONS.md).
import { contributions } from "@/ext";
import { isDocsPage } from "@/features/docs/route";
import { rootElement } from "./root";

/** Boot the editor (`invite`: handed to the extensions' "web.boot"). */
const startApp = (invite: string | null) => void import("./app").then((m) => m.runApp(invite));

const route = contributions("web.route").find((r) => r.match(location));
if (route) void route.mount(rootElement(), startApp);
else if (isDocsPage(location.pathname)) void import("@/features/docs/mount").then((m) => m.mountDocsPage(rootElement()));
else startApp(null);
