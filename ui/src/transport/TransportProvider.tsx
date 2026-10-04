import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { playheadStore } from "@/state/playhead";
import { useProjectStore } from "@/state/projectStore";
import { cmd } from "./cmd";
import { TransportContext, type ConnectionStatus } from "./context";
import { TransportSwitchContext, type TransportSwitch } from "./createDefaultTransport";
import type { EngineTransport } from "./EngineTransport";

const CONNECTING: ConnectionStatus = { status: "connecting" };

export interface TransportProviderProps {
  /** The engine connection. Owned by the caller (the provider never disposes it). */
  transport: EngineTransport;
  children?: ReactNode;
}

/**
 * Provides the `EngineTransport` to the tree and keeps the UI stores in sync with it:
 * - connects on mount and loads the returned project (if one is open) into `useProjectStore`;
 * - mirrors `ProjectLoaded` / `Patch` / `Transport` / `Project` (list, saved,
 *   dirty) / `Recording::ArmChanged` events into the store (refetching the whole project on
 *   a revision gap);
 * - forwards playhead and meter streams to `playheadStore`.
 */
export function TransportProvider({ transport: local, children }: TransportProviderProps) {
  const [remote, setRemote] = useState<EngineTransport | null>(null);
  const transport = remote ?? local;
  // Status is tagged with its transport, so swapping the transport reads as "connecting".
  const [state, setState] = useState<{ transport: EngineTransport; connection: ConnectionStatus } | null>(null);
  const connection: ConnectionStatus = state?.transport === transport ? state.connection : CONNECTING;

  useEffect(() => {
    let active = true;
    const store = useProjectStore.getState;
    const setConnection = (c: ConnectionStatus) => setState({ transport, connection: c });

    const refetch = () => {
      transport.send(cmd("Project", { type: "Get" })).then(
        (reply) => {
          if (active && reply.type === "Project") store().loadProject(reply.project);
        },
        (error: unknown) => console.error("Ethereal: failed to refetch the project", error),
      );
    };

    // Subscribe BEFORE connecting so nothing emitted during connect is lost.
    const offEvent = transport.onEvent((event) => {
      switch (event.type) {
        case "ProjectLoaded":
          // A (re)opened project starts out of safe mode; `SafeMode` follows if not.
          store().setSafeMode(false, []);
          store().loadProject(event.project);
          break;
        case "Project":
          switch (event.event.type) {
            case "ListChanged":
              store().setProjects(event.event.projects);
              break;
            case "Saved":
              store().upsertProjectSummary(event.event.project);
              break;
            case "DirtyChanged":
              store().setDirty(event.event.dirty);
              break;
            case "SafeMode":
              store().setSafeMode(event.event.active, event.event.devices);
              break;
          }
          break;
        case "Recording":
          if (event.event.type === "ArmChanged") store().setArmedTracks(event.event.armed);
          break;
        case "Patch":
          if (store().applyPatch(event.patch) === "gap") refetch();
          break;
        case "Transport":
          store().setTransport(event.state);
          break;
        default:
          // Plugin/Media/Engine/Notification: consumed by features via useTransportEvent.
          break;
      }
    });
    const offPlayhead = transport.subscribePlayhead((frame) => playheadStore.setPlayhead(frame));
    const offMeters = transport.subscribeMeters((frame) => playheadStore.setMeters(frame));

    transport.connect().then(
      (project) => {
        if (!active) return;
        // base-131: hosts open nothing on launch, so there may be no project yet.
        if (project) store().loadProject(project);
        else store().clearProject();
        setConnection({ status: "connected" });
      },
      (error: unknown) => {
        if (!active) return;
        console.error("Ethereal: failed to connect to the engine", error);
        setConnection({ status: "error", error });
      },
    );

    return () => {
      active = false;
      offEvent();
      offPlayhead();
      offMeters();
    };
  }, [transport]);

  // The provider owns the remote transport: dispose it when it is replaced or on unmount.
  useEffect(() => (remote ? () => remote.dispose() : undefined), [remote]);
  const switchToRemote = useCallback(
    (next: EngineTransport) => {
      // Silence the local engine while the remote one is in use.
      local.send(cmd("Transport", { type: "Stop" })).catch(() => {});
      setRemote(next);
    },
    [local],
  );
  const switchToLocal = useCallback(() => setRemote(null), []);
  const switcher = useMemo<TransportSwitch>(() => ({ local, remote, switchToRemote, switchToLocal }), [local, remote, switchToRemote, switchToLocal]);

  const value = useMemo(() => ({ transport, connection }), [transport, connection]);
  return (
    <TransportSwitchContext.Provider value={switcher}>
      <TransportContext.Provider value={value}>{children}</TransportContext.Provider>
    </TransportSwitchContext.Provider>
  );
}
