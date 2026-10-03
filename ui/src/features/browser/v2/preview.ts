/**
 * Preview state of the indexed browser: `../preview.ts` with library items. Indexed items
 * preview through `Browser::Preview { item, sync }` (tempo-synced); files without an item
 * (project media), files not indexed yet (`NotFound`) and engines without the command
 * (`Unsupported`) use `Media::Preview`.
 * Rows reset on `PreviewEnded` matching their source, exactly like the folder browser.
 */
import { useCallback, useState } from "react";
import type { Command, MediaSource, ReplyValue } from "@/generated";
import { errorMessage, useEngineEvent } from "@/features/transport-bar/engine";
import { cmd, isCommandFailed, type EngineTransport } from "@/transport";
import { sameSource } from "../preview";

type Send = (command: Command) => Promise<ReplyValue | undefined>;

export interface PreviewTarget {
  /** Row key (unique in the shown list). */
  key: string;
  source: MediaSource;
  /** Library item id, when indexed. */
  item: string | null;
}

export interface ItemPreview {
  previewing: string | null;
  toggle(target: PreviewTarget): Promise<void>;
  play(target: PreviewTarget): Promise<void>;
}

export function useItemPreview(transport: EngineTransport | null, send: Send, sync: boolean, onError: (message: string) => void): ItemPreview {
  const [current, setCurrent] = useState<{ key: string; source: MediaSource } | null>(null);

  useEngineEvent((event) => {
    if (event.type !== "Media" || event.event.type !== "PreviewEnded") return;
    const ended = event.event.source;
    setCurrent((c) => (c && sameSource(c.source, ended) ? null : c));
  });

  const start = useCallback(
    async (t: PreviewTarget): Promise<boolean> => {
      if (t.item && transport) {
        try {
          await transport.send(cmd("Browser", { type: "Preview", item: t.item, sync }));
          return true;
        } catch (e) {
          // Old engines, and folder rows the index has not reached yet: plain preview.
          if (!isCommandFailed(e, "Unsupported") && !isCommandFailed(e, "NotFound")) {
            onError(errorMessage(e));
            return false;
          }
        }
      }
      return !!(await send(cmd("Media", { type: "Preview", source: t.source })));
    },
    [transport, send, sync, onError],
  );

  const play = useCallback(
    async (t: PreviewTarget) => {
      if (current?.key === t.key) return;
      const next = { key: t.key, source: t.source };
      setCurrent(next);
      if (!(await start(t))) setCurrent((c) => (c === next ? null : c));
    },
    [current, start],
  );

  const toggle = useCallback(
    async (t: PreviewTarget) => {
      if (current?.key !== t.key) return play(t);
      setCurrent(null);
      await send(cmd("Media", { type: "StopPreview" }));
    },
    [current, play, send],
  );

  return { previewing: current?.key ?? null, toggle, play };
}
