/**
 * Accepting sample-browser drops. The payload format (MIME type, JSON shape, import helper)
 * is owned by the browser feature: see `ui/src/features/browser/dragPayload.ts`.
 */
export {
  BROWSER_DRAG_MIME,
  hasBrowserDrag,
  readBrowserDrag,
  resolveDroppedMedia,
  type BrowserDragPayload,
} from "@/features/browser/dragPayload";
