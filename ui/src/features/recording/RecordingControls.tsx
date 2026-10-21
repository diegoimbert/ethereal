import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { AudioDeviceList, Command, Event, InputList, ReplyValue, Track } from "@/generated";
import { Circle, MoreHorizontal } from "lucide-react";
import { Button, DismissibleError, IconButton, Popover, Select } from "@/kit";
import { EngineStatusIndicator, useCompactTopBar } from "@/features/transport-bar";
import { tracksOrdered, useProjectStore, useRecordSupport } from "@/state";
import { cmd, commandErrorText, isCommandFailed, TransportContext, type EngineTransport } from "@/transport";
import { useLiveRecordingFeed } from "./live/liveStore";
import {
  COUNT_IN_CHOICES,
  countInLabel,
  inputOptions,
  inputValue,
  isRecordable,
  midiChannelOptions,
  midiChannelValue,
  MONITOR_LABELS,
  MONITOR_MODES,
  withMidiChannel,
} from "./inputs";
import { tapArmBlocked } from "./tapRecording";

import { unsupportedTooltip } from "./support";
import { useT } from "@/i18n";

export { unsupportedTooltip };

type Support = { state: "unknown" } | { state: "supported"; inputs: InputList } | { state: "unsupported"; reason: string };

const EMPTY_TRACKS: Track[] = [];

function message(e: unknown): string {
  if (isCommandFailed(e)) return commandErrorText(e.error);
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
 * Recording controls: the recording indicator, count-in, punch in/out (loop region), and an
 * inputs panel (input device, per-track arm, input and monitoring). The record button is the
 * transport bar's; when the host has no inputs (`ListInputs` → `Unsupported`, web) these
 * controls tell it (`useRecordSupport`): it is disabled with that reason as its tooltip.
 */
export function RecordingControls() {
  const t = useT();
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
  // A narrow window: count-in, punch and the engine status in a "more" menu.
  const compact = useCompactTopBar();

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
  // The top bar's record button (the one record button) reads why it can't record.
  const unsupportedReason = unsupported ? unsupportedTooltip() : null;
  useEffect(() => {
    useRecordSupport.getState().setUnsupported(unsupportedReason);
  }, [unsupportedReason]);

  const countInAndPunch = (
    <>
      <label className="eth-rec__field" title={t("recording.count_in_title")}>
        <span>{t("recording.count_in")}</span>
        <Select
          size="sm"
          aria-label={t("recording.count_in")}
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
        aria-label={t("recording.punch_aria")}
        title={t("recording.punch_title")}
        active={punch}
        disabled={disabled || unsupported}
        onClick={() => void send(cmd("Recording", { type: "SetPunch", enabled: !punch }))}
      >
        {t("recording.punch")}
      </Button>
    </>
  );

  return (
    <div className="eth-rec" data-feature="recording" role="toolbar" aria-label={t("recording.toolbar")}>
      <span className="eth-rec__indicator" data-testid="recording-indicator" data-active={recording} role="status" aria-live="polite">
        {recording ? (
          <>
            <span className="eth-rec__dot" aria-hidden />
            {t("recording.rec")}
          </>
        ) : null}
      </span>
      {compact ? (
        <Popover
          placement="bottom-end"
          aria-label={t("recording.more")}
          className="eth-rec__more-popover"
          trigger={(p) => <IconButton {...p} size="sm" tone="ghost" label={t("recording.more")} icon={<MoreHorizontal />} data-testid="recording-more" />}
        >
          <div className="eth-rec__more">
            {countInAndPunch}
            <EngineStatusIndicator />
          </div>
        </Popover>
      ) : (
        countInAndPunch
      )}
      <Popover
        open={open && !!transport}
        onOpenChange={setOpen}
        placement="bottom-end"
        role="presentation"
        className="eth-rec__popover"
        trigger={(p) => (
          <Button
            {...p}
            size="sm"
            aria-label={t("recording.inputs_aria")}
            title={t("recording.inputs_title")}
            active={open}
            disabled={disabled}
            data-tour="rec-inputs"
          >
            {t("recording.inputs_button")}
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
        <DismissibleError className="eth-rec__error" onDismiss={clearError}>
          {error}
        </DismissibleError>
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
  const t = useT();
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
    <div className="eth-rec__panel" role="dialog" aria-label={t("recording.panel")} data-tour="rec-inputs-panel">
      {unsupported ? (
        <p className="eth-rec__note">{t("recording.unsupported_note")}</p>
      ) : (
        <label className="eth-rec__field">
          <span>{t("recording.audio_input")}</span>
          <Select
            size="sm"
            aria-label={t("recording.audio_input_device")}
            value={devices?.current.input_device ?? ""}
            onChange={(v) => void setDevice(v)}
            options={[
              { value: "", label: t("recording.none") },
              ...(devices?.inputs ?? []).map((d) => ({ value: d.name, label: d.name, userText: true })),
              ...(devices?.current.input_device && !devices.inputs.some((d) => d.name === devices.current.input_device)
                ? [{ value: devices.current.input_device, label: devices.current.input_device, userText: true }]
                : []),
            ]}
          />
        </label>
      )}
      {tracks.length === 0 ? (
        <p className="eth-rec__note">{t("recording.no_tracks")}</p>
      ) : (
        <table className="eth-rec__tracks">
          <thead>
            <tr>
              <th>{t("recording.col.track")}</th>
              <th>{t("recording.col.arm")}</th>
              <th>{t("recording.col.input")}</th>
              <th>{t("recording.col.monitor")}</th>
            </tr>
          </thead>
          <tbody>
            {tracks.map((tr) => {
              const isArmed = armed.includes(tr.id);
              const armBlocked = tapArmBlocked(transport.kind, tr.input, isArmed);
              const options = inputOptions(tr.kind, inputs, tr.input);
              return (
                <tr key={tr.id} data-track={tr.id}>
                  <td className="eth-rec__track-name eth-user-text">{tr.name}</td>
                  <td>
                    {/* A disabled button shows no tooltip in every browser: the wrapper carries it too. */}
                    <span title={armBlocked ?? undefined}>
                      <Button
                        size="sm"
                        aria-label={t("recording.arm_track", { name: tr.name })}
                        title={armBlocked ?? undefined}
                        active={isArmed}
                        className="eth-rec__arm"
                        disabled={armBlocked !== null}
                        onClick={() => void send(cmd("Recording", { type: "Arm", track: tr.id, armed: !isArmed, exclusive: false }))}
                      >
                        <Circle />
                      </Button>
                    </span>
                  </td>
                  <td>
                    <span className="eth-rec__input">
                      <Select
                        size="sm"
                        aria-label={t("recording.input_of", { name: tr.name })}
                        value={inputValue(tr.input)}
                        onChange={(v) => {
                          const option = options.find((o) => o.value === v);
                          if (option) void send(cmd("Recording", { type: "SetInput", track: tr.id, input: option.input }));
                        }}
                        options={options.map((o) => ({ value: o.value, label: o.label, userText: o.userText }))}
                      />
                      {tr.input.type === "Midi" && (
                        <Select
                          size="sm"
                          aria-label={t("recording.midi_channel_of", { name: tr.name })}
                          value={midiChannelValue(tr.input)}
                          onChange={(v) => void send(cmd("Recording", { type: "SetInput", track: tr.id, input: withMidiChannel(tr.input, v) }))}
                          options={midiChannelOptions()}
                        />
                      )}
                    </span>
                  </td>
                  <td>
                    <Select<Track["monitor"]>
                      size="sm"
                      aria-label={t("recording.monitoring_of", { name: tr.name })}
                      value={tr.monitor}
                      onChange={(v) => void send(cmd("Recording", { type: "SetMonitor", track: tr.id, monitor: v }))}
                      options={MONITOR_MODES.map((m) => ({ value: m, label: t(MONITOR_LABELS[m]) }))}
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
