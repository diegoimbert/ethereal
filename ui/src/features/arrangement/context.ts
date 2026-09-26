import { createContext, useContext, type RefObject } from "react";
import type { Clip, Command } from "@/generated";
import { useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, nextGestureId, type EngineTransport } from "@/transport";
import { isArrangementClip } from "./clipTime";
import type { Row } from "./layout";
import type { PeakCache } from "./peaks";

/** Shared plumbing of one mounted arrangement view. */
export interface ArrangementContextValue {
  transport: EngineTransport;
  peaks: PeakCache;
  /** The scrolling content element (rows + overlays); content px are relative to it. */
  contentRef: RefObject<HTMLDivElement | null>;
  /** Latest row layout. */
  rowsRef: RefObject<ReadonlyArray<Row>>;
  /** Focus the view so its keyboard shortcuts apply. */
  focus(): void;
}

export const ArrangementContext = createContext<ArrangementContextValue | null>(null);

export function useArrangement(): ArrangementContextValue {
  const ctx = useContext(ArrangementContext);
  if (!ctx) throw new Error("useArrangement() outside <ArrangementView>");
  return ctx;
}

/**
 * Send one edit as one undo gesture (the command carries a fresh gesture id, which is then
 * closed with `Edit::EndGesture`). Failures are logged: the mirror is unchanged.
 */
export async function sendEdit(transport: EngineTransport, command: Command | null): Promise<void> {
  if (!command) return;
  const gesture = nextGestureId();
  try {
    await transport.send(command, { gesture });
  } catch (err) {
    console.warn("arrangement edit failed", err);
  } finally {
    transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
  }
}

/** The selected arrangement clips, in start order. */
export function selectedClips(): Clip[] {
  const project = useProjectStore.getState().project;
  if (!project) return [];
  const out: Clip[] = [];
  for (const id of itemSelection.getState().selected.clip) {
    const c = project.clips[id];
    if (c && isArrangementClip(c)) out.push(c);
  }
  return out;
}
