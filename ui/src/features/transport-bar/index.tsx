// OWNERSHIP: the `ui-shell` node owns `ui/src/features/transport-bar/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `TransportBar`: keep this export name and keep it prop-less (read state via hooks).
import "./transport-bar.css";
import { useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { EngineStatus, GestureId, TimeSignaturePoint } from "@/generated";
import { Circle, Pause, Play, Redo2, Repeat, Square, Timer, Undo2 } from "lucide-react";
import { Button } from "@/kit";
import { timeSignaturePoints, useCpuLoad, usePlayhead, useProjectStore } from "@/state";
import { inputSettings } from "@/timeline";
import { cmd, nextGestureId } from "@/transport";
import { firstMatch, useShortcutLabel } from "@/features/keymap";
import { CaptureButton } from "@/features/capture";
import { midiTarget } from "@/features/midi-learn/targets";
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
 * Shortcuts (ignored while typing in a text field; keymap actions `transport.play`,
 * `edit.undo`, `edit.redo`): by default Space = play/stop, Ctrl/Cmd+Z = undo,
 * Ctrl/Cmd+Shift+Z or Ctrl+Y = redo.
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

  // Hold-and-drag on the tempo field: 1 BPM per 2 px up (whole BPM), Shift: 0.01 BPM steps.
  // The whole drag is one gesture, so one undo step.
  const tempoDrag = useRef<{ start: number; last: number; gesture: GestureId } | null>(null);
  const tempoDragHandlers = {
    onStart: () => {
      tempoDrag.current = { start: bpm, last: bpm, gesture: nextGestureId() };
    },
    onMove: (up: number, fine: boolean) => {
      const d = tempoDrag.current;
      if (!d) return;
      const raw = fine ? d.start + up * 0.05 : d.start + up * 0.5;
      const next = clampBpm(fine ? Math.round(raw * 100) / 100 : Math.round(raw));
      if (next === d.last) return;
      d.last = next;
      void send(cmd("Transport", { type: "SetTempo", bpm: next }), { gesture: d.gesture });
    },
    onEnd: () => {
      const d = tempoDrag.current;
      tempoDrag.current = null;
      if (d && d.last !== d.start) void send(cmd("Edit", { type: "EndGesture", gesture: d.gesture }));
    },
  };

  const playKey = useShortcutLabel("transport.play");
  useTransportShortcuts({
    enabled: !disabled,
    // Engine-side toggle: correct even if the last Transport event hasn't rendered yet.
    togglePlay: () => void send(cmd("Transport", { type: "TogglePlay" })),
    undo,
    redo,
  });

  return (
    <div className="eth-tb" data-feature="transport-bar" role="toolbar" aria-label="Transport">
      <div className="eth-tb__side" />

      {/* The capsule: transport, position and tempo. */}
      <div className="eth-tb__capsule">
        <div className="eth-tb__group">
          <Button
            tone="ghost"
            aria-label={playing ? "Stop" : "Play"}
            title={`${playing ? "Stop" : "Play"}${playKey ? ` (${playKey})` : ""}`}
            active={playing}
            className="eth-tb__btn eth-tb__play"
            {...midiTarget({ type: "Transport", action: "TogglePlay" })}
            disabled={disabled}
            onClick={togglePlay}
          >
            {playing ? <Pause aria-hidden /> : <Play aria-hidden />}
          </Button>
          <Button
            tone="ghost"
            aria-label="Stop"
            title="Stop (press again to return to start)"
            className="eth-tb__btn"
            {...midiTarget({ type: "Transport", action: "Stop" })}
            disabled={disabled}
            onClick={() => void send(cmd("Transport", { type: "Stop" }))}
          >
            <Square aria-hidden />
          </Button>
          <Button
            tone="ghost"
            aria-label="Record"
            title="Arrangement record"
            active={recording}
            className="eth-tb__btn eth-tb__record"
            {...midiTarget({ type: "Transport", action: "ToggleRecord" })}
            disabled={disabled}
            onClick={() => void send(cmd("Recording", { type: "SetRecording", enabled: !recording }))}
          >
            <Circle aria-hidden />
          </Button>
          {/* capture-midi: always listening; turns what was just played into a clip. */}
          <CaptureButton className="eth-tb__btn" />
        </div>

        <span className="eth-tb__divider" aria-hidden />

        <PositionDisplay />

        <span className="eth-tb__divider" aria-hidden />

        <div className="eth-tb__group eth-tb__tempo">
          <CommitField
            label="Tempo"
            title="Tempo in BPM (drag up/down or ↑/↓ to change, Shift for fine steps)"
            width="6ch"
            className="eth-tb-field--pill"
            disabled={disabled}
            value={formatBpm(bpm)}
            onCommit={(text) => {
              const value = parseBpm(text);
              if (value === null) return false;
              setTempo(value);
            }}
            onStep={(dir, fine) => setTempo(bpm + dir * (fine ? 0.1 : 1))}
            drag={tempoDragHandlers}
          />
          <span className="eth-tb__unit">BPM</span>
          <Button
            size="sm"
            tone="ghost"
            className="eth-tb__tap"
            {...midiTarget({ type: "Transport", action: "TapTempo" })}
            title="Tap tempo"
            disabled={disabled}
            onClick={() => void send(cmd("Transport", { type: "TapTempo" }))}
          >
            TAP
          </Button>
          <CommitField
            label="Time signature"
            title="Time signature at the playhead (e.g. 7/8)"
            width="5ch"
            className="eth-tb-field--pill"
            disabled={disabled}
            value={formatSignature(signature)}
            onCommit={(text) => {
              const sig = parseSignature(text);
              if (!sig) return false;
              void send(cmd("Transport", { type: "SetTimeSignature", signature: sig }));
            }}
          />
        </div>

        <span className="eth-tb__divider" aria-hidden />

        <div className="eth-tb__group">
          <Button
            tone="ghost"
            aria-label="Loop"
            title="Loop"
            className="eth-tb__btn eth-tb__toggle"
            {...midiTarget({ type: "Transport", action: "ToggleLoop" })}
            active={state?.loop_enabled ?? false}
            disabled={disabled}
            onClick={() => void send(cmd("Transport", { type: "SetLoopEnabled", enabled: !(state?.loop_enabled ?? false) }))}
          >
            <Repeat aria-hidden />
          </Button>
          <Button
            tone="ghost"
            aria-label="Metronome"
            title="Metronome"
            className="eth-tb__btn eth-tb__toggle"
            {...midiTarget({ type: "Transport", action: "ToggleMetronome" })}
            active={state?.metronome ?? false}
            disabled={disabled}
            onClick={() => void send(cmd("Transport", { type: "SetMetronome", enabled: !(state?.metronome ?? false) }))}
          >
            <Timer aria-hidden />
          </Button>
        </div>
      </div>

      <div className="eth-tb__side eth-tb__side--end">
        {error && (
          <button type="button" className="eth-tb__error" role="alert" title="Dismiss" onClick={clearError}>
            {error}
          </button>
        )}
        <div className="eth-tb__group">
          <Button
            tone="ghost"
            aria-label="Undo"
            className="eth-tb__btn"
            title={history.undo_label ? `Undo ${history.undo_label}` : "Nothing to undo"}
            disabled={disabled || !history.can_undo}
            onClick={undo}
          >
            <Undo2 aria-hidden />
          </Button>
          <Button
            tone="ghost"
            aria-label="Redo"
            className="eth-tb__btn"
            title={history.redo_label ? `Redo ${history.redo_label}` : "Nothing to redo"}
            disabled={disabled || !history.can_redo}
            onClick={redo}
          >
            <Redo2 aria-hidden />
          </Button>
        </div>
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
      // keymap: chords from the user's keymap (defaults Space, Mod+Z, Mod+Shift+Z / Ctrl+Y).
      const action = firstMatch(["transport.play", "edit.undo", "edit.redo"] as const, e);
      if (action === "transport.play") {
        // A focused button would also "click" on Space; let it.
        if (e.key === " " && e.target instanceof HTMLButtonElement) return;
        e.preventDefault();
        h.togglePlay();
      } else if (action === "edit.undo") {
        e.preventDefault();
        undoIfPossible(h);
      } else if (action === "edit.redo") {
        e.preventDefault();
        redoIfPossible(h);
      }
    };
    // Mouse side buttons (Settings > Input): Back = undo, Forward = redo when enabled. The
    // press is swallowed then, so the webview never navigates back/forward.
    const sideButton = (e: MouseEvent) => (e.button === 3 || e.button === 4) && inputSettings().sideButtons === "undoRedo";
    const onSideDown = (e: MouseEvent) => {
      if (sideButton(e)) e.preventDefault();
    };
    const onSideUp = (e: MouseEvent) => {
      const h = ref.current;
      if (!sideButton(e)) return;
      e.preventDefault();
      if (!h.enabled) return;
      if (e.button === 3) undoIfPossible(h);
      else redoIfPossible(h);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onSideDown);
    window.addEventListener("mouseup", onSideUp);
    window.addEventListener("auxclick", onSideDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onSideDown);
      window.removeEventListener("mouseup", onSideUp);
      window.removeEventListener("auxclick", onSideDown);
    };
  }, []);
}
