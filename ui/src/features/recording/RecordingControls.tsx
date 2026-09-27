import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { AudioDeviceList, Command, Event, InputList, ReplyValue, Track } from "@/generated";
import { Button, Popover, Select } from "@/kit";
import { tracksOrdered, useProjectStore } from "@/state";
import { cmd, isCommandFailed, TransportContext, type EngineTransport } from "@/transport";
import { useLiveRecordingFeed } from "./live/liveStore";
import { COUNT_IN_CHOICES, countInLabel, inputOptions, inputValue, isRecordable, MONITOR_MODES } from "./inputs";

/** Shown on the disabled record button when the host has no inputs (browser build). */
export const UNSUPPORTED_TOOLTIP = "Recording needs the desktop app: audio and MIDI inputs aren't available in the browser";

type Support =
  | { state: "unknown" }
  | { state: "supported"; inputs: InputList }
  | { state: "unsupported"; reason: string };

const EMPTY_TRACKS: Track[] = [];

function message(e: unknown): string {
  if (isCommandFailed(e)) return e.error.message || e.error.code;
  return e instanceof Error ? e.message : String(e);
}

/** `send` that reports failures in `error` instead of throwing (and works without a provider). */
function useSend(transport: EngineTransport | null) {
  const [error, setError] = useState<string | null>(null);
  const send = useCallback(
    async (command: Command): Promise<ReplyValue | undefined> => {
      if (!transport) return undefined;
      try {
        const reply = await transport.send(command);
        setError(null);
        return reply;
      } catch (e) {
        setError(message(e));
        return undefined;
      }
    },
    [transport],
  );
  return { send, error, clearError: useCallback(() => setError(null), []) };
}

function useEvents(transport: EngineTransport | null, listener: (e: Event) => void): void {
  const ref = useRef(listener);
  useEffect(() => {
    ref.current = listener;
  });
  useEffect(() => transport?.onEvent((e) => ref.current(e)), [transport]);
}

/**
 * Recording controls: record button + indicator, count-in, punch in/out (loop region), and
 * an inputs panel (input device, per-track arm, input and monitoring). The record button is
 * disabled with a tooltip when the host has no inputs (`ListInputs` → `Unsupported`, web).
 */
export function RecordingControls() {
  const ctx = useContext(TransportContext);
  const transport = ctx?.transport ?? null;
  // Keep the live recording view fed even while no arrangement lane is mounted.
  useLiveRecordingFeed(transport);
  const connected = ctx?.connection.status === "connected";
  const hasProject = useProjectStore((s) => s.project !== null);
  const recording = useProjectStore((s) => s.transport?.recording ?? false);
  const countIn = useProjectStore((s) => s.project?.settings.count_in_bars ?? 0);
  const { send, error, clearError } = useSend(transport);
  const [support, setSupport] = useState<Support>({ state: "unknown" });
  const [punch, setPunch] = useState(false);
  const [open, setOpen] = useState(false);

  // Bumped to list the inputs again (after the input device changed).
  const [inputsVersion, setInputsVersion] = useState(0);

  useEffect(() => {
    if (!transport || !connected || !hasProject) return;
    let active = true;
    transport.send(cmd("Recording", { type: "ListInputs" })).then(
      (reply) => {
        if (active && reply.type === "Inputs") setSupport({ state: "supported", inputs: reply.inputs });
      },
      (e: unknown) => {
        if (!active) return;
        if (isCommandFailed(e) && e.error.code === "Unsupported") setSupport({ state: "unsupported", reason: message(e) });
        else setSupport({ state: "supported", inputs: { audio: [], midi: [] } });
      },
    );
    return () => {
      active = false;
    };
  }, [transport, connected, hasProject, inputsVersion]);

  useEvents(transport, (e) => {
    if (e.type !== "Recording") return;
    if (e.event.type === "PunchChanged") setPunch(e.event.enabled);
    else if (e.event.type === "InputsChanged") setSupport({ state: "supported", inputs: e.event.inputs });
  });

  const unsupported = support.state === "unsupported";
  const disabled = !transport || !hasProject;
  const recordTitle = unsupported ? UNSUPPORTED_TOOLTIP : recording ? "Stop recording" : "Record armed tracks (count-in, punch)";

  return (
    <div className="eth-rec" data-feature="recording" role="toolbar" aria-label="Recording">
      <span className="eth-rec__indicator" data-testid="recording-indicator" data-active={recording} role="status" aria-live="polite">
        {recording ? (
          <>
            <span className="eth-rec__dot" aria-hidden />
            REC
          </>
        ) : null}
      </span>
      {/* A disabled button shows no tooltip in every browser: the wrapper carries it too. */}
      <span title={recordTitle} className="eth-rec__record-wrap">
        <Button
          aria-label="Record armed tracks"
          title={recordTitle}
          active={recording}
          className="eth-rec__record"
          disabled={disabled || unsupported}
          onClick={() => void send(cmd("Recording", { type: "SetRecording", enabled: !recording }))}
        >
          ●
        </Button>
      </span>
      <label className="eth-rec__field" title="Count-in before recording (pre-roll)">
        <span>Count-in</span>
        <Select
          size="sm"
          aria-label="Count-in"
          disabled={disabled}
          value={String(countIn)}
          onChange={(v) => void send(cmd("Recording", { type: "SetCountIn", bars: Number(v) }))}
          options={(COUNT_IN_CHOICES.includes(countIn) ? COUNT_IN_CHOICES : [...COUNT_IN_CHOICES, countIn]).map((bars) => ({
            value: String(bars),
            label: countInLabel(bars),
          }))}
        />
      </label>
      <Button
        size="sm"
        aria-label="Punch in/out"
        title="Punch in/out: record only inside the loop region"
        active={punch}
        disabled={disabled || unsupported}
        onClick={() => void send(cmd("Recording", { type: "SetPunch", enabled: !punch }))}
      >
        PUNCH
      </Button>
      <Popover
        open={open && !!transport}
        onOpenChange={setOpen}
        placement="bottom-end"
        role="presentation"
        className="eth-rec__popover"
        trigger={(t) => (
          <Button
            {...t}
            size="sm"
            aria-label="Inputs"
            title="Inputs and monitoring"
            active={open}
            disabled={disabled}
          >
            IN ▾
          </Button>
        )}
      >
        {transport && (
          <InputsPanel
            transport={transport}
            inputs={support.state === "supported" ? support.inputs : null}
            unsupported={unsupported}
            send={send}
            onDeviceChanged={() => setInputsVersion((v) => v + 1)}
          />
        )}
      </Popover>
      {error && (
        <button type="button" className="eth-rec__error" role="alert" title="Dismiss" onClick={clearError}>
          {error}
        </button>
      )}
    </div>
  );
}

interface InputsPanelProps {
  transport: EngineTransport;
  inputs: InputList | null;
  unsupported: boolean;
  send(command: Command): Promise<ReplyValue | undefined>;
  onDeviceChanged(): void;
}

function InputsPanel({ transport, inputs, unsupported, send, onDeviceChanged }: InputsPanelProps) {
  const tracks = useProjectStore(useShallow((s) => (s.project ? tracksOrdered(s.project).filter((t) => isRecordable(t.kind)) : EMPTY_TRACKS)));
  const armed = useProjectStore((s) => s.armedTracks);
  const [devices, setDevices] = useState<AudioDeviceList | null>(null);

  useEffect(() => {
    if (unsupported) return;
    let active = true;
    transport.send(cmd("Engine", { type: "ListAudioDevices" })).then(
      (reply) => {
        if (active && reply.type === "AudioDevices") setDevices(reply.devices);
      },
      () => undefined,
    );
    return () => {
      active = false;
    };
  }, [transport, unsupported]);

  const setDevice = async (name: string) => {
    const reply = await send(
      cmd("Engine", {
        type: "SetAudioConfig",
        config: { backend: null, host: null, output_device: null, input_device: name, sample_rate: null, buffer_size: null },
      }),
    );
    if (reply) {
      setDevices((d) => (d ? { ...d, current: { ...d.current, input_device: name || null } } : d));
      onDeviceChanged();
    }
  };

  return (
    <div className="eth-rec__panel" role="dialog" aria-label="Recording inputs">
      {unsupported ? (
        <p className="eth-rec__note">{UNSUPPORTED_TOOLTIP}.</p>
      ) : (
        <label className="eth-rec__field">
          <span>Audio input</span>
          <Select
            size="sm"
            aria-label="Audio input device"
            value={devices?.current.input_device ?? ""}
            onChange={(v) => void setDevice(v)}
            options={[
              { value: "", label: "None" },
              ...(devices?.inputs ?? []).map((d) => ({ value: d.name, label: d.name })),
              ...(devices?.current.input_device && !devices.inputs.some((d) => d.name === devices.current.input_device)
                ? [{ value: devices.current.input_device, label: devices.current.input_device }]
                : []),
            ]}
          />
        </label>
      )}
      {tracks.length === 0 ? (
        <p className="eth-rec__note">No audio or MIDI tracks.</p>
      ) : (
        <table className="eth-rec__tracks">
          <thead>
            <tr>
              <th>Track</th>
              <th>Arm</th>
              <th>Input</th>
              <th>Monitor</th>
            </tr>
          </thead>
          <tbody>
            {tracks.map((t) => {
              const isArmed = armed.includes(t.id);
              const options = inputOptions(t.kind, inputs, t.input);
              return (
                <tr key={t.id} data-track={t.id}>
                  <td className="eth-rec__track-name">{t.name}</td>
                  <td>
                    <Button
                      size="sm"
                      aria-label={`Arm ${t.name}`}
                      active={isArmed}
                      className="eth-rec__arm"
                      onClick={() => void send(cmd("Recording", { type: "Arm", track: t.id, armed: !isArmed, exclusive: false }))}
                    >
                      ●
                    </Button>
                  </td>
                  <td>
                    <Select
                      size="sm"
                      aria-label={`Input of ${t.name}`}
                      value={inputValue(t.input)}
                      onChange={(v) => {
                        const option = options.find((o) => o.value === v);
                        if (option) void send(cmd("Recording", { type: "SetInput", track: t.id, input: option.input }));
                      }}
                      options={options.map((o) => ({ value: o.value, label: o.label }))}
                    />
                  </td>
                  <td>
                    <Select<Track["monitor"]>
                      size="sm"
                      aria-label={`Monitoring of ${t.name}`}
                      value={t.monitor}
                      onChange={(v) => void send(cmd("Recording", { type: "SetMonitor", track: t.id, monitor: v }))}
                      options={MONITOR_MODES.map((m) => ({ value: m, label: m }))}
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
