// Public entry of `@ethereal/ui`, consumed by apps/web and apps/desktop hosts.
// Hosts import this package's *source* (no prebuilt lib), so their Vite config must alias
// `@` → `ui/src` as well (see apps/web/vite.config.ts).
export { App, type DetailTabId, type MainViewId, type SidebarTabId } from "./app/App";
export * from "./kit";
