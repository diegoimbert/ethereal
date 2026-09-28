/**
 * OS file drops (Finder / Explorer / a file manager) onto the app's drop zones: the
 * arrangement lanes and the browser panel.
 *
 * - Web, remote UI, and desktop fallback: an HTML5 drop carrying `File`s; they are uploaded.
 * - Desktop (macOS): the shell forwards the dropped **paths** (`PathDropHost.onPathDrop`)
 *   instead of letting the page see the drop; the page still gets `dragover`s, so the zones
 *   show their hover hints. The path drop lands in the zone hovered last, at the last
 *   hovered point (`noteHover`), and the engine imports the paths itself.
 */
import type { DragEvent } from "react";
import type { EngineTransport, Unsubscribe } from "@/transport";
import { fileSource, pathSource, type ImportSource } from "./sources";

/** The desktop transport's path-drop and file-dialog hooks (`TauriTransport`). */
export interface PathDropHost {
  /** OS paths dropped on the window (macOS). */
  onPathDrop(listener: (paths: string[]) => void): Unsubscribe;
  /** The OS file dialog: absolute paths, or `null` when dismissed. */
  pickAudioFiles(): Promise<string[] | null>;
}

export function isPathDropHost(t: EngineTransport | null | undefined): t is EngineTransport & PathDropHost {
  const h = t as Partial<PathDropHost> | null | undefined;
  return typeof h?.onPathDrop === "function" && typeof h.pickAudioFiles === "function";
}

/** Minimal `DataTransfer` surface (tests pass plain objects). */
export interface FileDragData {
  types: ReadonlyArray<string>;
  files?: ArrayLike<File> | null;
}

/** Does this drag carry OS files? (During `dragover` only the types are readable.) */
export function hasOsFiles(dt: FileDragData | null | undefined): boolean {
  return !!dt && Array.from(dt.types).includes("Files");
}

/** The dropped files, as import sources. */
export function droppedFiles(dt: FileDragData | null | undefined): ImportSource[] {
  return Array.from(dt?.files ?? []).map((f) => fileSource(f));
}

/** A point in a drop zone, as the zone's own drop handler reads it. */
export interface DropPoint {
  clientX: number;
  clientY: number;
  altKey: boolean;
}

export type ZoneHandler = (sources: ImportSource[], point: DropPoint) => void;

interface Hover {
  handler: ZoneHandler;
  point: DropPoint;
  time: number;
}

/** A path drop arriving later than this after the last hover is ignored. */
export const HOVER_STALE_MS = 3000;

let lastHover: Hover | null = null;

/** Called by a zone on every OS-file `dragover` it accepts. */
export function noteHover(e: Pick<DragEvent, "clientX" | "clientY" | "altKey">, handler: ZoneHandler, now = Date.now()): void {
  lastHover = { handler, point: { clientX: e.clientX, clientY: e.clientY, altKey: e.altKey }, time: now };
}

/** Deliver dropped OS paths to the zone hovered last. `false` if none is recent. */
export function deliverPathDrop(paths: readonly string[], now = Date.now()): boolean {
  const h = lastHover;
  lastHover = null;
  if (!h || now - h.time > HOVER_STALE_MS || paths.length === 0) return false;
  h.handler(paths.map(pathSource), h.point);
  return true;
}

/** Test hook. */
export function resetHover(): void {
  lastHover = null;
}
