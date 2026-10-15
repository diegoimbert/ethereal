/**
 * Browser sample preview state (`media-preview`). `Media::Preview` starts a preview (the
 * engine replaces any playing one), `Media::StopPreview` stops it; the engine answers with
 * `MediaEvent::PreviewEnded { source, reason }` exactly once per preview (finished, stopped,
 * replaced or failed), which resets the previewing row (CONTRACTS.md §11.15).
 *
 * Rows carry a generation (the row object of one `play`): stopping then re-previewing the
 * same file quickly must not let the first preview's late `Stopped`/`Replaced` reset the
 * second one. See {@link endsRow}.
 */
import { useCallback, useRef, useState } from "react";
import type { Command, DirectoryEntry, MediaSource, PreviewEndReason, ReplyValue } from "@/generated";
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
    case "Path":
      return b.type === "Path" && a.path === b.path;
  }
}

/**
 * Whether the `PreviewEnded` `ended` resets `row`. `pending` is the row whose `Preview`
 * command has not replied yet: the controller ends the previous preview (`Stopped` by a
 * `StopPreview`, `Replaced` by a `Preview`) before it replies, so until then a `Stopped` or
 * `Replaced` of the same source is an earlier preview's (the row's own preview can only be
 * stopped or replaced by a later command, which resets the row itself). `Finished` and
 * `Failed` always apply.
 */
export function endsRow(row: { source: MediaSource } | null, pending: unknown, ended: { source: MediaSource; reason: PreviewEndReason }): boolean {
  if (!row || !sameSource(row.source, ended.source)) return false;
  return row !== pending || ended.reason === "Finished" || ended.reason === "Failed";
}

type Send = (command: Command) => Promise<ReplyValue | undefined>;

export interface BrowserPreview {
  /** Path of the row being previewed (`DirectoryEntry.path`), if any. */
  previewing: string | null;
  /** Preview `entry`, or stop it if it is the one playing. */
  toggle(entry: DirectoryEntry, source: MediaSource): Promise<void>;
  /** Preview `entry` (keeps it playing if it already is). */
  play(entry: DirectoryEntry, source: MediaSource): Promise<void>;
}

export function useBrowserPreview(send: Send): BrowserPreview {
  const [current, setCurrent] = useState<{ path: string; source: MediaSource } | null>(null);
  const pending = useRef<unknown>(null);

  useEngineEvent((event) => {
    if (event.type !== "Media" || event.event.type !== "PreviewEnded") return;
    const ended = event.event;
    // Read now: the updater runs later, when this command may have replied.
    const inFlight = pending.current;
    // The end of an older (replaced or stopped) preview never resets a newer one.
    setCurrent((c) => (endsRow(c, inFlight, ended) ? null : c));
  });

  const play = useCallback(
    async (entry: DirectoryEntry, source: MediaSource) => {
      if (current?.path === entry.path) return;
      // Current before the reply: the old preview's `Replaced` (sent before the reply) then
      // doesn't match it, and a `Failed` of this one does.
      const next = { path: entry.path, source };
      setCurrent(next);
      pending.current = next;
      const reply = await send(cmd("Media", { type: "Preview", source }));
      if (pending.current === next) pending.current = null;
      if (!reply) setCurrent((c) => (c === next ? null : c));
    },
    [current, send],
  );

  const toggle = useCallback(
    async (entry: DirectoryEntry, source: MediaSource) => {
      if (current?.path !== entry.path) return play(entry, source);
      setCurrent(null);
      await send(cmd("Media", { type: "StopPreview" }));
    },
    [current, play, send],
  );

  return { previewing: current?.path ?? null, toggle, play };
}
