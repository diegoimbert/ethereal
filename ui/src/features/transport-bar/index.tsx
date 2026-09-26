// OWNERSHIP: the `ui-shell` node owns `ui/src/features/transport-bar/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `TransportBar`: keep this export name and keep it prop-less (read state via hooks).
import "./transport-bar.css";
import { useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { EngineStatus, TimeSignaturePoint } from "@/generated";
import { Button } from "@/kit";
import { timeSignaturePoints, useCpuLoad, usePlayhead, useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { CommitField } from "./CommitField";
import { isTextEntry, useEngineCommands, useEngineEvent, useOptionalConnection, useOptionalTransport } from "./engine";
import {
  barPosition,
  clampBpm,
  formatBarPosition,
  formatBpm,
  formatCpu,
  formatSeconds,
  formatSignature,
  parseBpm,
  parseSignature,
} from "./format";

const EMPTY_POINTS: TimeSignaturePoint[] = [];

/**
 * Transport bar: play/stop/record, loop, metronome, tempo (+ tap), time signature, position
 * (bars and time), undo/redo, CPU load and engine status.
 *
 * Shortcuts (ignored while typing in a text field): Space = play/stop,
 * Ctrl/Cmd+Z = undo, Ctrl/Cmd+Shift+Z or Ctrl+Y = redo.
 */
export function TransportBar() {
  const { transport, send, error, clearError } = useEngineCommands();
  const state = useProjectStore((s) => s.transport);
  const history = useProjectStore((s) => s.history);
  const hasProject = useProjectStore((s) => s.project !== null);
  const disabled = !transport || !hasProject;

  const playing = state?.playing ?? false;
  const recording = state?.recording ?? false;
  const bpm = state?.bpm ?? 120;
  const signature = state?.time_signature ?? { numerator: 4, denominator: 4 };

  const togglePlay = () => void send(cmd("Transport", { type: playing ? "Stop" : "Play" }));
  const undo = () => void send(cmd("Edit", { type: "Undo" }));
  const redo = () => void send(cmd("Edit", { type: "Redo" }));
  const setTempo = (value: number) => void send(cmd("Transport", { type: "SetTempo", bpm: clampBpm(value) }));

  useTransportShortcuts({
    enabled: !disabled,
    // Engine-side toggle: correct even if the last Transport event hasn't rendered yet.
    togglePlay: () => void send(cmd("Transport", { type: "TogglePlay" })),
    undo,
    redo,
  });

  return (
    <div className="eth-tb" data-feature="transport-bar" role="toolbar" aria-label="Transport">
      <div className="eth-tb__group">
        <Button
          aria-label={playing ? "Stop" : "Play"}
          title={playing ? "Stop (Space)" : "Play (Space)"}
          active={playing}
          className="eth-tb__play"
          disabled={disabled}
          onClick={togglePlay}
        >
          {playing ? "■" : "▶"}
        </Button>
        <Button
          aria-label="Stop"
          title="Stop (press again to return to start)"
          disabled={disabled}
          onClick={() => void send(cmd("Transport", { type: "Stop" }))}
        >
          ⏹
        </Button>
        <Button
          aria-label="Record"
          title="Arrangement record"
          active={recording}
          className="eth-tb__record"
          disabled={disabled}
          onClick={() => void send(cmd("Recording", { type: "SetRecording", enabled: !recording }))}
        >
          ●
        </Button>
      </div>

      <div className="eth-tb__group">
        <Button
          aria-label="Loop"
          title="Loop"
          active={state?.loop_enabled ?? false}
          disabled={disabled}
          onClick={() => void send(cmd("Transport", { type: "SetLoopEnabled", enabled: !(state?.loop_enabled ?? false) }))}
        >
          ⟳
        </Button>
        <Button
          aria-label="Metronome"
          title="Metronome"
          active={state?.metronome ?? false}
          disabled={disabled}
          onClick={() => void send(cmd("Transport", { type: "SetMetronome", enabled: !(state?.metronome ?? false) }))}
        >
          ♩
        </Button>
      </div>

      <div className="eth-tb__group">
        <CommitField
          label="Tempo"
          title="Tempo in BPM (↑/↓ to nudge, Shift for 0.1)"
          width="6ch"
          disabled={disabled}
          value={formatBpm(bpm)}
          onCommit={(text) => {
            const value = parseBpm(text);
            if (value === null) return false;
            setTempo(value);
          }}
          onStep={(dir, fine) => setTempo(bpm + dir * (fine ? 0.1 : 1))}
        />
        <span className="eth-tb__unit">BPM</span>
        <Button size="sm" title="Tap tempo" disabled={disabled} onClick={() => void send(cmd("Transport", { type: "TapTempo" }))}>
          TAP
        </Button>
        <CommitField
          label="Time signature"
          title="Time signature at the playhead (e.g. 7/8)"
          width="5ch"
          disabled={disabled}
          value={formatSignature(signature)}
          onCommit={(text) => {
            const sig = parseSignature(text);
            if (!sig) return false;
            void send(cmd("Transport", { type: "SetTimeSignature", signature: sig }));
          }}
        />
      </div>

      <PositionDisplay />

      <div className="eth-tb__group">
        <Button
          aria-label="Undo"
          title={history.undo_label ? `Undo ${history.undo_label}` : "Nothing to undo"}
          disabled={disabled || !history.can_undo}
          onClick={undo}
        >
          ↶
        </Button>
        <Button
          aria-label="Redo"
          title={history.redo_label ? `Redo ${history.redo_label}` : "Nothing to redo"}
          disabled={disabled || !history.can_redo}
          onClick={redo}
        >
          ↷
        </Button>
      </div>

      {error && (
        <button type="button" className="eth-tb__error" role="alert" title="Dismiss" onClick={clearError}>
          {error}
        </button>
      )}

      <div className="eth-tb__group eth-tb__group--end">
        <CpuMeter />
        <EngineStatusIndicator />
      </div>
    </div>
  );
}

/** Bars.beats.sixteenths and minutes:seconds of the playhead (re-renders at playhead rate). */
function PositionDisplay() {
  const frame = usePlayhead();
  const points = useProjectStore(useShallow((s) => (s.project ? timeSignaturePoints(s.project) : EMPTY_POINTS)));
  const beats = frame?.transport.position ?? 0;
  const seconds = frame?.transport.seconds ?? 0;
  return (
    <div className="eth-tb__position" aria-label="Position">
      <output className="eth-tb__bars" data-testid="position-bars">
        {formatBarPosition(barPosition(beats, points))}
      </output>
      <output className="eth-tb__time" data-testid="position-time">
        {formatSeconds(seconds)}
      </output>
    </div>
  );
}

function CpuMeter() {
  const load = useCpuLoad();
  return (
    <span className="eth-tb__cpu" title="Engine DSP load" data-testid="cpu">
      CPU {formatCpu(load)}
    </span>
  );
}

/** Connection + audio backend status (dot), details in the tooltip. */
function EngineStatusIndicator() {
  const connection = useOptionalConnection();
  const transport = useOptionalTransport();
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const connected = connection?.status === "connected";

  useEffect(() => {
    if (!transport || !connected) return;
    let active = true;
    transport.send(cmd("Engine", { type: "GetStatus" })).then(
      (reply) => {
        if (active && reply.type === "Status") setStatus(reply.status);
      },
      () => undefined,
    );
    return () => {
      active = false;
    };
  }, [transport, connected]);

  useEngineEvent((event) => {
    if (event.type !== "Engine") return;
    if (event.event.type === "Status") setStatus(event.event.status);
    else {
      const xruns = event.event.count;
      setStatus((s) => (s ? { ...s, xruns } : s));
    }
  });

  let level: "ok" | "warn" | "error" | "off";
  let text: string;
  if (!connection) {
    level = "off";
    text = "No engine";
  } else if (connection.status === "connecting") {
    level = "warn";
    text = "Connecting…";
  } else if (connection.status === "error") {
    level = "error";
    text = "Engine connection failed";
  } else if (status && !status.running) {
    level = "error";
    text = `Audio stopped (${status.backend})`;
  } else {
    level = status && status.xruns > 0 ? "warn" : "ok";
    text = status
      ? `${status.backend} · ${status.sample_rate} Hz · ${status.buffer_size} smp · ${status.xruns} dropouts`
      : "Connected";
  }

  return (
    <span className="eth-tb__engine" data-level={level} data-testid="engine-status" title={text} aria-label={`Engine: ${text}`}>
      <span className="eth-tb__engine-dot" aria-hidden />
      {status && <span className="eth-tb__engine-text">{status.backend}</span>}
    </span>
  );
}

interface ShortcutHandlers {
  enabled: boolean;
  togglePlay(): void;
  undo(): void;
  redo(): void;
}

// Shortcuts read state from the store at key time, not from the last render: a key pressed
// right after an edit must see it even if React hasn't re-rendered (or run effects) yet.
function undoIfPossible(h: ShortcutHandlers): void {
  if (useProjectStore.getState().history.can_undo) h.undo();
}

function redoIfPossible(h: ShortcutHandlers): void {
  if (useProjectStore.getState().history.can_redo) h.redo();
}

function useTransportShortcuts(handlers: ShortcutHandlers): void {
  const ref = useRef(handlers);
  useEffect(() => {
    ref.current = handlers;
  });
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const h = ref.current;
      if (!h.enabled || e.defaultPrevented || isTextEntry(e.target)) return;
      const mod = e.metaKey || e.ctrlKey;
      if (e.key === " " && !mod && !e.altKey) {
        // A focused button would also "click" on Space; let it.
        if (e.target instanceof HTMLButtonElement) return;
        e.preventDefault();
        h.togglePlay();
      } else if (mod && !e.altKey && e.key.toLowerCase() === "z") {
        e.preventDefault();
        if (e.shiftKey) redoIfPossible(h);
        else undoIfPossible(h);
      } else if (e.ctrlKey && !e.metaKey && !e.shiftKey && e.key.toLowerCase() === "y") {
        e.preventDefault();
        redoIfPossible(h);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
