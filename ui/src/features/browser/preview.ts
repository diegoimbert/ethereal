/**
 * Browser sample preview state (`media-preview`). `Media::Preview` starts a preview (the
 * engine replaces any playing one), `Media::StopPreview` stops it; the engine answers with
 * `MediaEvent::PreviewEnded { source, reason }` exactly once per preview (finished, stopped,
 * replaced or failed), which resets the previewing row (CONTRACTS.md §11.15).
 */
import { useCallback, useState } from "react";
import type { Command, DirectoryEntry, MediaSource, ReplyValue } from "@/generated";
import { useEngineEvent } from "@/features/transport-bar/engine";
import { cmd } from "@/transport";

/** Same preview source (field-wise; sources are small tagged objects). */
export function sameSource(a: MediaSource, b: MediaSource): boolean {
  switch (a.type) {
    case "Location":
      return (
        b.type === "Location" &&
        a.path === b.path &&
        a.location.type === b.location.type &&
        (a.location.type !== "Library" || (b.location.type === "Library" && a.location.id === b.location.id))
      );
    case "Project":
      return b.type === "Project" && a.media === b.media;
    case "Upload":
      return b.type === "Upload" && a.upload === b.upload;
  }
}

type Send = (command: Command) => Promise<ReplyValue | undefined>;

export interface BrowserPreview {
  /** Path of the row being previewed (`DirectoryEntry.path`), if any. */
  previewing: string | null;
  /** Preview `entry`, or stop it if it is the one playing. */
  toggle(entry: DirectoryEntry, source: MediaSource): Promise<void>;
}

export function useBrowserPreview(send: Send): BrowserPreview {
  const [current, setCurrent] = useState<{ path: string; source: MediaSource } | null>(null);

  useEngineEvent((event) => {
    if (event.type !== "Media" || event.event.type !== "PreviewEnded") return;
    const ended = event.event.source;
    // The end of an older (replaced) preview never resets a newer one.
    setCurrent((c) => (c && sameSource(c.source, ended) ? null : c));
  });

  const toggle = useCallback(
    async (entry: DirectoryEntry, source: MediaSource) => {
      if (current?.path === entry.path) {
        setCurrent(null);
        await send(cmd("Media", { type: "StopPreview" }));
        return;
      }
      // Current before the reply: the old preview's `Replaced` (sent before the reply) then
      // doesn't match it, and a `Failed` of this one does.
      const next = { path: entry.path, source };
      setCurrent(next);
      const reply = await send(cmd("Media", { type: "Preview", source }));
      if (!reply) setCurrent((c) => (c === next ? null : c));
    },
    [current, send],
  );

  return { previewing: current?.path ?? null, toggle };
}
