import { useEffect, useState } from "react";
import type { Command, InputList, ReplyValue, Track } from "@/generated";
import { DismissibleError, Select } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, commandErrorText, isCommandFailed, useTransport, useTransportEvent } from "@/transport";
import { useT } from "@/i18n";
import { inputOptions, inputValue, midiChannelOptions, midiChannelValue, MONITOR_LABELS, MONITOR_MODES, withMidiChannel } from "./inputs";
import { unsupportedTooltip } from "./support";

type Send = (command: Command) => Promise<ReplyValue | undefined>;

/** A track's input: the input (or MIDI port), and a MIDI input's channel. */
export function TrackInputSelects({ track, inputs, send }: { track: Track; inputs: InputList | null; send: Send }) {
  const t = useT();
  const options = inputOptions(track.kind, inputs, track.input);
  return (
    <span className="eth-rec__input">
      <Select
        size="sm"
        aria-label={t("recording.input_of", { name: track.name })}
        value={inputValue(track.input)}
        onChange={(v) => {
          const option = options.find((o) => o.value === v);
          if (option) void send(cmd("Recording", { type: "SetInput", track: track.id, input: option.input }));
        }}
        options={options.map((o) => ({ value: o.value, label: o.label, userText: o.userText }))}
      />
      {track.input.type === "Midi" && (
        <Select
          size="sm"
          aria-label={t("recording.midi_channel_of", { name: track.name })}
          value={midiChannelValue(track.input)}
          onChange={(v) => void send(cmd("Recording", { type: "SetInput", track: track.id, input: withMidiChannel(track.input, v) }))}
          options={midiChannelOptions()}
        />
      )}
    </span>
  );
}

/** A track's monitoring (auto, in, off). */
export function TrackMonitorSelect({ track, send }: { track: Track; send: Send }) {
  const t = useT();
  return (
    <Select<Track["monitor"]>
      size="sm"
      aria-label={t("recording.monitoring_of", { name: track.name })}
      value={track.monitor}
      onChange={(v) => void send(cmd("Recording", { type: "SetMonitor", track: track.id, monitor: v }))}
      options={MONITOR_MODES.map((m) => ({ value: m, label: t(MONITOR_LABELS[m]) }))}
    />
  );
}

/**
 * One track's recording setup, its input and monitoring (the inputs panel's row for it): what
 * a right-click on a track's arm button shows. Lists the inputs itself; a host without inputs
 * (web) says why instead.
 */
export function TrackRecordSetup({ track: id }: { track: Track["id"] }) {
  const t = useT();
  const transport = useTransport();
  const track = useProjectStore((s) => s.project?.tracks[id]);
  const [inputs, setInputs] = useState<InputList | null>(null);
  const [unsupported, setUnsupported] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    transport.send(cmd("Recording", { type: "ListInputs" })).then(
      (reply) => {
        if (active && reply.type === "Inputs") setInputs(reply.inputs);
      },
      (e: unknown) => {
        if (active && isCommandFailed(e) && e.error.code === "Unsupported") setUnsupported(true);
      },
    );
    return () => {
      active = false;
    };
  }, [transport]);
  useTransportEvent((e) => {
    if (e.type === "Recording" && e.event.type === "InputsChanged") setInputs(e.event.inputs);
  });

  const send: Send = async (command) => {
    try {
      const reply = await transport.send(command);
      setError(null);
      return reply;
    } catch (e) {
      setError(isCommandFailed(e) ? commandErrorText(e.error) : e instanceof Error ? e.message : String(e));
      return undefined;
    }
  };

  if (!track) return null;
  return (
    <div className="eth-rec__setup">
      {unsupported ? (
        <p className="eth-rec__note">{unsupportedTooltip()}</p>
      ) : (
        <>
          <span className="eth-rec__setup-label">{t("recording.col.input")}</span>
          <span className="eth-rec__setup-label">{t("recording.col.monitor")}</span>
          <TrackInputSelects track={track} inputs={inputs} send={send} />
          <TrackMonitorSelect track={track} send={send} />
        </>
      )}
      {error && (
        <DismissibleError className="eth-rec__error" onDismiss={() => setError(null)}>
          {error}
        </DismissibleError>
      )}
    </div>
  );
}
