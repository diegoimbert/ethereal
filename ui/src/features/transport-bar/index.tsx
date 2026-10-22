// The app shell (ui/src/app/App.tsx) mounts `TransportBar`: keep it prop-less (read state via hooks).
import { contributions } from "@/ext";
import "./transport-bar.css";
import { useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { EngineStatus, GestureId, TimeSignaturePoint } from "@/generated";
import { Circle, Metronome, Pause, Play, Redo2, Repeat, Square, Undo2 } from "lucide-react";
import { Button, DismissibleError, Menu, Popover } from "@/kit";
import { notify } from "@/features/notifications";
import { timeSignaturePoints, useCpuLoad, usePlayhead, useProjectStore, useReadOnly, useRecordSupport } from "@/state";
import { inputSettings } from "@/timeline";
import { cmd, nextGestureId } from "@/transport";
import { chordsFor, dispatchPaused, firstMatch, useKeymapStore, useShortcutLabel } from "@/features/keymap";
import { playOrStopHere, playOrStopToStart, stopButton } from "@/features/time-edits/marker";
import { asKeyboardEvent, chordCodes, onPluginWindowKey, setEditorKeyCapture } from "@/features/plugin-keys";
import { CaptureButton } from "@/features/capture";
import { KeyboardMidiButton } from "@/features/keyboard-midi";
import { MetronomePanel } from "@/features/tempo";
import { midiTarget } from "@/features/midi-learn/targets";
import { tText, useT } from "@/i18n";
import { CommitField } from "./CommitField";
import { diskTitle, useDiskStore } from "./disk";
import { errorMessage, isTextEntry, useEngineCommands, useEngineEvent, useOptionalConnection, useOptionalTransport } from "./engine";
import { blurAfterPointer, installTransportKeys, transportActionFor, type TransportKeyAction } from "./keys";
import {
  barPosition,
  clampBpm,
  formatBarPosition,
  formatBpm,
  formatCpu,
  formatSeconds,
  formatSecondsShort,
  formatSignature,
  parseBpm,
  parseSignature,
} from "./format";
import { useCompactTopBar } from "./compact";
import { useProjectLoading } from "./loading";
import { LoadingStatus } from "./LoadingStatus";
import { useMobileUi } from "@/features/mobile/mode";

export { COMPACT_TOP_BAR_QUERY, useCompactTopBar } from "./compact";
export { loadingText, projectLoading, refuseWhileLoading, useProjectLoading } from "./loading";
export { LoadingStatus } from "./LoadingStatus";

const EMPTY_POINTS: TimeSignaturePoint[] = [];

/**
 * Transport bar: play/stop/record, loop, metronome, tempo (+ tap), time signature, position
 * (bars and time), undo/redo, CPU load, disk-streaming dropouts and engine status.
 *
 * Shortcuts (keymap actions): Space = play / stop back to where playback started, Shift+Space
 * = play / stop where the playhead is (`transport.play`, `transport.stopToSelection`: everywhere but text fields and
 * open menus, also from plugin editor windows; see `keys.ts`), Ctrl/Cmd+Z = undo,
 * Ctrl/Cmd+Shift+Z or Ctrl+Y = redo (ignored while typing in a text field).
 *
 * Stop: stops; when already stopped, returns to the selection cursor, then to the start.
 */
export function TransportBar() {
  const { transport, send, error, clearError } = useEngineCommands();
  const state = useProjectStore((s) => s.transport);
  const history = useProjectStore((s) => s.history);
  const t = useT();
  const hasProject = useProjectStore((s) => s.project !== null);
  const disabled = !transport || !hasProject;
  // A shared project's viewer: play, stop, loop and the position; tempo and signature shown, not set.
  const readOnly = useReadOnly();
  // The project is still loading (its audio is suspended): no play, stop or record yet,
  // shortcuts included. Everything else works (the project can be explored and edited).
  const loading = useProjectLoading();
  // An extension says nothing here can play now (e.g. joined with no engine): the reason.
  const extBlocked = useTransportBlocked();
  const blocked = loading ? t("error.project.loading") : extBlocked;
  const compact = useCompactTopBar();
  const recordUnsupported = useRecordSupport((s) => s.unsupported);
  // The metronome's settings: a right-click on its button.
  const [metronomeMenu, setMetronomeMenu] = useState(false);

  const playing = state?.playing ?? false;
  const recording = state?.recording ?? false;
  const bpm = state?.bpm ?? 120;
  const signature = state?.time_signature ?? { numerator: 4, denominator: 4 };
  useDiskDropouts(playing);

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
  const runTransportKey = (action: TransportKeyAction) => {
    if (!transport) return;
    // Both play; stopping, Space returns to where playback started, Shift+Space stays.
    if (action === "transport.play") playOrStopToStart(transport);
    else playOrStopHere(transport);
  };
  useTransportShortcuts({ enabled: !disabled, undo, redo });
  useTransportKeys({ enabled: !disabled && !blocked, run: runTransportKey });
  const transportOff = disabled || !!loading;

  return (
    <div className="eth-tb" data-feature="transport-bar" role="toolbar" aria-label={t("shell.transport.toolbar")}>
      <div className="eth-tb__side" />

      {/* The capsule: transport, position and tempo. */}
      <div className="eth-tb__capsule">
        <div className="eth-tb__group">
          <Button
            tone="ghost"
            aria-label={playing ? t("shell.transport.stop") : t("shell.transport.play")}
            title={
              blocked
                ? blocked
                : playKey
                  ? t(playing ? "shell.transport.stop_key" : "shell.transport.play_key", { key: playKey })
                  : t(playing ? "shell.transport.stop" : "shell.transport.play")
            }
            active={playing}
            className="eth-tb__btn eth-tb__play"
            data-tour="play"
            {...midiTarget({ type: "Transport", action: "TogglePlay" })}
            disabled={transportOff || (!!blocked && !playing)}
            onClick={togglePlay}
          >
            {playing ? <Pause aria-hidden /> : <Play aria-hidden />}
          </Button>
          <Button
            tone="ghost"
            aria-label={t("shell.transport.stop")}
            title={loading ? t("error.project.loading") : t("shell.transport.stop_title")}
            data-testid="transport-stop"
            className="eth-tb__btn"
            {...midiTarget({ type: "Transport", action: "Stop" })}
            disabled={transportOff}
            onClick={() => transport && stopButton(transport)}
          >
            <Square aria-hidden />
          </Button>
          {!readOnly && (
            // The record button (the tour's "record-button"). A host that can't record (no inputs,
            // web) disables it with the reason; a disabled button shows no tooltip in every
            // browser, so the wrapper carries it too.
            <span className="eth-tb__record-wrap" title={recordUnsupported ?? undefined} data-tour="record-button">
              <Button
                tone="ghost"
                aria-label={t("shell.transport.record")}
                title={recordUnsupported ?? blocked ?? t("shell.transport.record_title")}
                active={recording}
                className="eth-tb__btn eth-tb__record"
                {...midiTarget({ type: "Transport", action: "ToggleRecord" })}
                disabled={transportOff || !!recordUnsupported || (!!blocked && !recording)}
                onClick={() => void send(cmd("Recording", { type: "SetRecording", enabled: !recording }))}
              >
                <Circle aria-hidden />
              </Button>
            </span>
          )}
          {/* capture-midi: always listening; turns what was just played into a clip. */}
          {!readOnly && <CaptureButton className="eth-tb__btn" />}
          {/* keyboard-midi: play notes from the computer keyboard (M). */}
          {!readOnly && <KeyboardMidiButton className="eth-tb__btn" />}
          {loading && <LoadingStatus progress={loading} compact={compact} />}
        </div>

        <span className="eth-tb__divider" aria-hidden />

        <PositionDisplay />

        <span className="eth-tb__divider" aria-hidden />

        <div className="eth-tb__group eth-tb__tempo">
          <CommitField
            label={t("shell.transport.tempo")}
            title={t("shell.transport.tempo_title")}
            width="6ch"
            className="eth-tb-field--pill"
            disabled={disabled || readOnly}
            value={formatBpm(bpm)}
            onCommit={(text) => {
              const value = parseBpm(text);
              if (value === null) return false;
              setTempo(value);
            }}
            onStep={(dir, fine) => setTempo(bpm + dir * (fine ? 0.1 : 1))}
            drag={tempoDragHandlers}
          />
          <span className="eth-tb__unit">bpm</span>
          <CommitField
            label={t("shell.transport.signature")}
            title={t("shell.transport.signature_title")}
            width="5ch"
            className="eth-tb-field--pill"
            disabled={disabled || readOnly}
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
            aria-label={t("shell.transport.loop")}
            title={t("shell.transport.loop")}
            className="eth-tb__btn eth-tb__toggle"
            {...midiTarget({ type: "Transport", action: "ToggleLoop" })}
            active={state?.loop_enabled ?? false}
            disabled={disabled}
            onClick={() => void send(cmd("Transport", { type: "SetLoopEnabled", enabled: !(state?.loop_enabled ?? false) }))}
          >
            <Repeat aria-hidden />
          </Button>
          {!readOnly && (
            <Popover
              open={metronomeMenu}
              onOpenChange={setMetronomeMenu}
              placement="bottom-start"
              aria-label={t("tempo.metronome.settings")}
              trigger={() => (
                <Button
                  tone="ghost"
                  aria-label={t("shell.transport.metronome")}
                  title={t("shell.transport.metronome_title")}
                  className="eth-tb__btn eth-tb__toggle"
                  data-testid="metronome-toggle"
                  {...midiTarget({ type: "Transport", action: "ToggleMetronome" })}
                  active={state?.metronome ?? false}
                  disabled={disabled}
                  onClick={() => void send(cmd("Transport", { type: "SetMetronome", enabled: !(state?.metronome ?? false) }))}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    if (!disabled) setMetronomeMenu(true);
                  }}
                >
                  <Metronome aria-hidden />
                </Button>
              )}
            >
              <MetronomePanel />
            </Popover>
          )}
        </div>
      </div>

      <div className="eth-tb__side eth-tb__side--end">
        {error && (
          <DismissibleError className="eth-tb__error" onDismiss={clearError}>
            {error}
          </DismissibleError>
        )}
        {!readOnly && (
          <div className="eth-tb__group">
            <Button
              tone="ghost"
              aria-label={t("shell.action.edit_undo")}
              className="eth-tb__btn"
              title={history.undo_label ? t("history.undo_named", { what: tText(history.undo_key, history.undo_label) }) : t("history.nothing_to_undo")}
              disabled={disabled || !history.can_undo}
              onClick={undo}
            >
              <Undo2 aria-hidden />
            </Button>
            <Button
              tone="ghost"
              aria-label={t("shell.action.edit_redo")}
              className="eth-tb__btn"
              title={history.redo_label ? t("history.redo_named", { what: tText(history.redo_key, history.redo_label) }) : t("history.nothing_to_redo")}
              disabled={disabled || !history.can_redo}
              onClick={redo}
            >
              <Redo2 aria-hidden />
            </Button>
          </div>
        )}
        {!readOnly && <CpuMeter />}
        {!readOnly && <DiskIndicator />}
        {/* Compact, in the recording controls' "more" menu. */}
        {!readOnly && !compact && <EngineStatusIndicator />}
      </div>
    </div>
  );
}

/**
 * Bars.beats.sixteenths and minutes:seconds of the playhead (re-renders at playhead rate).
 * Compact top bar: one of them, the time (hundredths) until a click switches to bars and back.
 */
export function PositionDisplay() {
  const t = useT();
  const frame = usePlayhead();
  const points = useProjectStore(useShallow((s) => (s.project ? timeSignaturePoints(s.project) : EMPTY_POINTS)));
  // The mobile shell keeps the plain display (no tap target this small on a touch screen).
  const mobile = useMobileUi((s) => s.enabled);
  const compact = useCompactTopBar() && !mobile;
  const [showBars, setShowBars] = useState(false);
  const beats = frame?.transport.position ?? 0;
  const seconds = frame?.transport.seconds ?? 0;
  if (compact) {
    return (
      <button
        type="button"
        className="eth-tb__position eth-tb__position--compact"
        aria-label={t("shell.transport.position")}
        title={showBars ? t("shell.transport.position_show_time") : t("shell.transport.position_show_bars")}
        onClick={() => setShowBars((b) => !b)}
      >
        <output className="eth-tb__bars" data-testid={showBars ? "position-bars" : "position-time"}>
          {showBars ? formatBarPosition(barPosition(beats, points)) : formatSecondsShort(seconds)}
        </output>
      </button>
    );
  }
  return (
    <div className="eth-tb__position" aria-label={t("shell.transport.position")}>
      <output className="eth-tb__bars" data-testid="position-bars">
        {formatBarPosition(barPosition(beats, points))}
      </output>
      <output className="eth-tb__time" data-testid="position-time">
        {formatSeconds(seconds)}
      </output>
    </div>
  );
}

/**
 * The engine's load; a click opens a menu that stops the engine (nothing plays, its plugins
 * unload and the audio device is free for other apps) or starts it again.
 */
function CpuMeter() {
  const t = useT();
  const load = useCpuLoad();
  const transport = useOptionalTransport();
  const [running, setRunning] = useState(true);
  useEngineEvent((e) => {
    if (e.type === "Engine" && e.event.type === "Running") setRunning(e.event.running);
  });
  const toggle = async () => {
    if (!transport) return;
    try {
      await transport.send(cmd("Engine", { type: "SetRunning", running: !running }));
    } catch (e) {
      notify("Error", errorMessage(e));
    }
  };
  return (
    <Menu
      aria-label={t("shell.transport.engine_menu")}
      items={[
        {
          id: "engine",
          label: running ? t("shell.transport.stop_engine") : t("shell.transport.start_engine"),
          onSelect: () => void toggle(),
        },
      ]}
      trigger={(p) => (
        <button
          type="button"
          className="eth-tb__cpu"
          title={running ? t("shell.transport.cpu_title") : t("shell.transport.engine_off_title")}
          data-testid="cpu"
          data-off={!running || undefined}
          disabled={!transport}
          {...p}
        >
          {running ? `cpu ${formatCpu(load)}` : t("shell.transport.engine_off")}
        </button>
      )}
    />
  );
}

/** Count `MediaEvent::StreamUnderruns`; each start of playback opens a new toast session. */
function useDiskDropouts(playing: boolean) {
  useEngineEvent((event) => useDiskStore.getState().onEvent(event));
  const wasPlaying = useRef(playing);
  useEffect(() => {
    if (playing && !wasPlaying.current) useDiskStore.getState().playbackStarted();
    wasPlaying.current = playing;
  }, [playing]);
}

/**
 * Disk-streaming dropouts: a warning light next to the CPU meter, only once streamed
 * media dropped out (hosts or projects that don't stream never show it). Click to reset.
 */
function DiskIndicator() {
  const t = useT();
  const dropouts = useDiskStore((s) => s.dropouts);
  const since = useDiskStore((s) => s.since);
  if (dropouts === 0) return null;
  const title = diskTitle(dropouts, since);
  return (
    <button type="button" className="eth-tb__disk" data-testid="disk-dropouts" title={title} aria-label={title} onClick={() => useDiskStore.getState().reset()}>
      <span className="eth-tb__disk-dot" aria-hidden />
      {t("shell.transport.disk", { count: dropouts })}
    </button>
  );
}

/** Connection + audio backend status (dot), details in the tooltip. */
export function EngineStatusIndicator() {
  const t = useT();
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
    else if (event.event.type === "Xrun") {
      const xruns = event.event.count;
      setStatus((s) => (s ? { ...s, xruns } : s));
    }
  });

  let level: "ok" | "warn" | "error" | "off";
  let text: string;
  if (!connection) {
    level = "off";
    text = t("shell.transport.engine_none");
  } else if (connection.status === "connecting") {
    level = "warn";
    text = t("shell.transport.engine_connecting");
  } else if (connection.status === "error") {
    level = "error";
    text = t("shell.transport.engine_failed");
  } else if (status?.device_released) {
    // "Play with: no engine" (or a participant's engine): the device is free for other apps.
    level = "off";
    text = t("shell.transport.engine_released");
  } else if (status && !status.running) {
    level = "error";
    text = t("shell.transport.engine_stopped", { backend: status.backend });
  } else {
    level = status && status.xruns > 0 ? "warn" : "ok";
    text = status
      ? t("shell.transport.engine_status", { backend: status.backend, rate: status.sample_rate, buffer: status.buffer_size, count: status.xruns })
      : t("shell.transport.engine_connected");
  }

  return (
    <span className="eth-tb__engine" data-level={level} data-testid="engine-status" title={text} aria-label={t("shell.transport.engine", { status: text })}>
      <span className="eth-tb__engine-dot" aria-hidden />
      {status && (
        // The audio system's name ("cpal", "mock") isn't translated.
        <span className="eth-tb__engine-text" data-i18n-exempt={status.device_released ? undefined : ""}>
          {status.device_released ? t("shell.transport.engine_closed") : status.backend}
        </span>
      )}
    </span>
  );
}

interface ShortcutHandlers {
  enabled: boolean;
  undo(): void;
  redo(): void;
}

/**
 * Space / Shift+Space (rebindable): always the transport, from the app window (capture
 * phase, before any focused control: `keys.ts`) and from plugin editor windows (forwarded
 * by the host, `features/plugin-keys`). Also hands the focus back after pointer clicks on
 * buttons, and asks plugin windows to forward the transport chords' keys.
 */
function useTransportKeys(handlers: { enabled: boolean; run(action: TransportKeyAction): void }): void {
  const ref = useRef(handlers);
  useEffect(() => {
    ref.current = handlers;
  });
  useEffect(
    () =>
      installTransportKeys({
        enabled: () => ref.current.enabled,
        paused: dispatchPaused,
        run: (action) => ref.current.run(action),
      }),
    [],
  );
  useEffect(() => blurAfterPointer(), []);
  const transport = useOptionalTransport();
  // Keys from plugin editor windows (Space always; rebound transport keys registered below).
  useEffect(() => {
    if (!transport) return;
    return onPluginWindowKey(transport, (key) => {
      if (!key.down || key.repeat || !ref.current.enabled) return;
      const action = transportActionFor(asKeyboardEvent(key));
      if (action) ref.current.run(action);
    });
  }, [transport]);
  const keymap = useKeymapStore((s) => s.keymap);
  useEffect(() => {
    if (!transport) return;
    setEditorKeyCapture(transport, "transport", chordCodes([...chordsFor("transport.play"), ...chordsFor("transport.stopToSelection")]));
  }, [transport, keymap]);
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
      // keymap: chords from the user's keymap (defaults Mod+Z, Mod+Shift+Z / Ctrl+Y). The
      // transport keys are `useTransportKeys`'.
      const action = firstMatch(["edit.undo", "edit.redo"] as const, e);
      if (action === "edit.undo") {
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

/** Why the transport can't play now, from the extensions (`@/ext` "transport.blocked"), or `null`. */
function useTransportBlocked(): string | null {
  // The contributions are fixed once the app runs: each is a hook, called in order.
  // eslint-disable-next-line react-hooks/rules-of-hooks -- hooks, fixed once the app runs
  const reasons = contributions("transport.blocked").map((useBlocked) => useBlocked());
  return reasons.find((r) => r !== null) ?? null;
}
