// Resolving missing plugins and audio files after an import, or any time (CONTRACTS.md
// §14.2): deterministic matches from the engine, optional AI suggestions through the AI
// chat's provider, one undo step. The app shell mounts `ImportResolverRoot`; menus add the
// "Resolve…" entries.
import "./resolver.css";

export { ImportResolverRoot } from "./ImportResolverRoot";
export { ResolverDialog } from "./ResolverDialog";
export { ReplacePopover } from "./ReplacePopover";
export {
  replaceDeviceLabel,
  resolveLabel,
  resolveMissingLabel,
  replaceDeviceEntries,
  resolveClipEntries,
  resolveMediaEntries,
  resolvePluginEntries,
  resolveReportEntries,
  resolverCommands,
  withResolveClipEntries,
} from "./menus";
export { catalogNow, isMissingPlugin, loadCatalog } from "./catalog";
export { acceptReplacement, closeReplace, isRackDevice, openReplace, resetReplace, useReplace } from "./replace";
export {
  applyResolveEvent,
  autoShow,
  closeResolver,
  openIfMissing,
  openResolver,
  proposeResolution,
  resetResolver,
  setAutoShow,
  useResolver,
  type Focus,
  type Section,
} from "./store";
export { MissingDeviceControls, MissingPluginsStatus } from "./MissingItems";
export { missingCount, useMissingCount } from "./missingCount";
