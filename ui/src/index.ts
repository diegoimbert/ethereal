// Public entry of `@ethereal/ui`, consumed by apps/web and apps/desktop hosts.
// Hosts import this package's *source* (no prebuilt lib), so their Vite config must alias
// `@` → `ui/src` as well (see apps/web/vite.config.ts).
export { App } from "./app/App";
export type { DrawerTab, LeftTab } from "./app/shell/shellStore";
export * from "./kit";
export * from "./state";
export * from "./transport";
