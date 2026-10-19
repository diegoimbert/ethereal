// The UI's selection for the agent tools (`Agent::SetSelection`): the in-app AI chat and MCP
// agents resolve "this clip", "the selected track" and ties between candidates with it. Sent
// when the selection changes (debounced), and again on every new engine connection.
import { useEffect } from "react";
import type { AgentSelection } from "@/generated";
import { useEditorStore } from "@/state/editor";
import { useSelectionStore } from "@/state/selection";
import { itemSelection } from "@/timeline/selection";
import { cmd, type EngineTransport } from "@/transport";

/** How long the selection must stay put before it is sent. */
export const SELECTION_SYNC_MS = 150;

/** The current selection, as the agent tools see it. */
export function agentSelection(): AgentSelection {
  const track = useSelectionStore.getState().selectedTrack;
  const sel = itemSelection.getState().selected;
  return {
    cursor: null,
    tracks: track ? [track] : [],
    clips: [...sel.clip],
    notes: [...sel.note],
    devices: [],
    editing_clip: useEditorStore.getState().clip,
  };
}

/** Keep the engine's copy of the selection current while `transport` is connected. */
export function useAgentSelectionSync(transport: EngineTransport | null, connected: boolean): void {
  useEffect(() => {
    if (!transport || !connected) return;
    let last = "";
    let timer: ReturnType<typeof setTimeout> | null = null;
    const send = () => {
      timer = null;
      const selection = agentSelection();
      const key = JSON.stringify(selection);
      if (key === last) return;
      last = key;
      void transport.send(cmd("Agent", { type: "SetSelection", selection })).catch(() => undefined);
    };
    const schedule = () => {
      if (timer === null) timer = setTimeout(send, SELECTION_SYNC_MS);
    };
    send();
    const offs = [useSelectionStore.subscribe(schedule), itemSelection.subscribe(schedule), useEditorStore.subscribe(schedule)];
    return () => {
      if (timer !== null) clearTimeout(timer);
      for (const off of offs) off();
    };
  }, [transport, connected]);
}
