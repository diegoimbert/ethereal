// Public entry of `@ethereal/ui`, consumed by apps/web and apps/desktop hosts.
// Hosts import this package's *source* (no prebuilt lib), so their Vite config must alias
// `@` → `ui/src` as well (see apps/web/vite.config.ts).
export { App } from "./app/App";
export { enableMobileUi } from "./features/mobile/mode";
export type { DrawerTab, LeftTab } from "@/state/shell";
export * from "./kit";
export * from "./state";
export * from "./transport";
