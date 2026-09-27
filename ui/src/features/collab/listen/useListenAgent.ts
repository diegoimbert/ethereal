// Runs the listener (receiver, stream clock → playhead) for one engine connection.
import { useEffect } from "react";
import { playheadStore } from "@/state/playhead";
import { useProjectStore } from "@/state/projectStore";
import { TempoMap } from "@/timeline/tempoMap";
import type { EngineTransport } from "@/transport";
import { ListenAgent } from "./agent";
import { send } from "./menu";
import { useListenStore } from "./store";

/** Tempo map of the replicated project, rebuilt only when its tempo/signature tables change. */
function tempoSource() {
  let key: unknown = null;
  let map: TempoMap | null = null;
  return () => {
    const p = useProjectStore.getState().project;
    if (!p) return null;
    if (key !== p.tempo_points || map === null) {
      key = p.tempo_points;
      map = TempoMap.fromProject(p);
    }
    return map;
  };
}

/** Run the listener (receiver, stream clock → playhead) for this engine connection. */
export function useListenAgent(transport: EngineTransport): void {
  useEffect(() => {
    useListenStore.getState().reset();
    const agent = new ListenAgent({
      send: send(transport),
      tempo: tempoSource(),
      setPlayhead: (frame) => playheadStore.setOverride(frame),
    });
    const off = transport.onEvent((e) => agent.onEvent(e));
    return () => {
      off();
      agent.dispose();
    };
  }, [transport]);
}
