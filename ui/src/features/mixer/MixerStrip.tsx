import clsx from "clsx";
import { useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import type { SendId, Track, TrackSend } from "@/generated";
import { Button, Fader, Knob, Meter } from "@/kit";
import { useProjectStore, useSelectionStore } from "@/state";
import { cmd, newId } from "@/transport";
import { useGestureSender, useSend, type GestureSender } from "@/features/devices/gesture";
import { formatDb, formatPan } from "@/features/devices/paramScale";
import { dbToFader, defaultOutputLabel, faderToDb, outputTargets, outputValue, parseOutputValue } from "./routing";
import { useMeterLevels } from "./useMeterLevels";

const FADER_HEIGHT = 120;
/** Sends top out at unity, like Ableton. */
const SEND_MAX_DB = 0;

function colorCss(color: number): string {
  return `#${color.toString(16).padStart(6, "0")}`;
}

function SendKnob({ track, ret, send, sender }: { track: Track; ret: Track; send: TrackSend | undefined; sender: GestureSender }) {
  // A send created during the current drag, before its patch arrived.
  const pending = useRef<SendId | null>(null);
  const level = send?.level ?? -144;
  const onChange = (n: number) => {
    const db = faderToDb(n, SEND_MAX_DB);
    const id = send?.id ?? pending.current;
    if (id) {
      void sender.send(cmd("Mixer", { type: "SetSendLevel", send: id, level: db }));
      return;
    }
    const created = newId();
    if (sender.dragging()) pending.current = created;
    void sender.send(cmd("Mixer", { type: "CreateSend", id: created, from: track.id, to: ret.id, level: db, pre_fader: false }));
  };
  return (
    <div className={clsx("eth-strip__send", !send && "eth-strip__send--off")} data-send-to={ret.id}>
      <Knob
        size={24}
        value={send ? dbToFader(level, SEND_MAX_DB) : 0}
        defaultValue={dbToFader(-144, SEND_MAX_DB)}
        label={`Send ${ret.name}`}
        valueText={formatDb(level)}
        onChange={onChange}
        onChangeStart={sender.begin}
        onChangeEnd={() => {
          pending.current = null;
          sender.end();
        }}
      />
      <span className="eth-strip__send-name" title={ret.name}>
        {ret.name.slice(0, 1)}
      </span>
      {send && (
        <Button
          size="sm"
          variant="ghost"
          active={send.pre_fader}
          aria-label={`Pre-fader send ${ret.name}`}
          title={send.pre_fader ? "Pre-fader (click for post)" : "Post-fader (click for pre)"}
          onClick={() =>
            void sender.send(cmd("Mixer", { type: "SetSendPreFader", send: send.id, pre_fader: !send.pre_fader }))
          }
        >
          {send.pre_fader ? "Pre" : "Post"}
        </Button>
      )}
    </div>
  );
}

function OutputSelect({ track }: { track: Track }) {
  const send = useSend();
  const targets = useProjectStore(useShallow((s) => (s.project ? outputTargets(s.project.tracks, track) : [])));
  const defaultLabel = useProjectStore((s) => (s.project ? defaultOutputLabel(s.project.tracks, track) : "Master"));
  return (
    <select
      className="eth-strip__output"
      aria-label={`${track.name} output`}
      value={outputValue(track.output)}
      onChange={(e) =>
        void send(cmd("Mixer", { type: "SetOutput", track: track.id, output: parseOutputValue(e.target.value) }))
      }
    >
      <option value="default">{defaultLabel}</option>
      {targets.map((t) => (
        <option key={t.id} value={`track:${t.id}`}>
          {t.name}
        </option>
      ))}
      <option value="none">No output</option>
    </select>
  );
}

function StripMeter({ track }: { track: Track }) {
  const { levels, clipped, resetClip } = useMeterLevels(track.id);
  return (
    <>
      <Meter levels={levels} height={FADER_HEIGHT} className="eth-strip__meter" />
      <button
        type="button"
        className={clsx("eth-strip__clip", clipped && "eth-strip__clip--on")}
        aria-label={clipped ? `${track.name} clipped (click to reset)` : `${track.name} clip indicator`}
        onClick={resetClip}
      />
    </>
  );
}

export interface MixerStripProps {
  track: Track;
  /** Return tracks (a send knob per return). */
  returns: ReadonlyArray<Track>;
  /** For group tracks: whether the children are folded. */
  folded?: boolean;
  onToggleFold?: () => void;
}

export function MixerStrip({ track, returns, folded, onToggleFold }: MixerStripProps) {
  const sender = useGestureSender();
  const sends = useProjectStore(
    useShallow((s) => (s.project ? Object.values(s.project.sends).filter((x) => x.from === track.id) : [])),
  );
  const selected = useSelectionStore((s) => s.selectedTrack === track.id);
  const selectTrack = useSelectionStore((s) => s.selectTrack);
  const { volume, pan, mute, solo } = track.mixer;
  const isMaster = track.kind === "Master";
  const sendTargets = isMaster ? [] : returns.filter((r) => r.id !== track.id);

  return (
    <div
      className={clsx("eth-strip", `eth-strip--${track.kind.toLowerCase()}`, selected && "eth-strip--selected")}
      data-track={track.id}
      role="group"
      aria-label={track.name}
      style={{ ["--eth-strip-color" as string]: colorCss(track.color) }}
    >
      <div className="eth-strip__header">
        {onToggleFold && (
          <Button
            size="sm"
            variant="ghost"
            className="eth-strip__fold"
            aria-expanded={!folded}
            aria-label={folded ? `Unfold ${track.name}` : `Fold ${track.name}`}
            onClick={onToggleFold}
          >
            {folded ? "▸" : "▾"}
          </Button>
        )}
        <button
          type="button"
          className="eth-strip__name"
          aria-pressed={selected}
          title={track.name}
          onClick={() => selectTrack(track.id)}
        >
          {track.name}
        </button>
      </div>

      <div className="eth-strip__routing">{!isMaster && <OutputSelect track={track} />}</div>

      <div className="eth-strip__sends">
        {sendTargets.map((r) => (
          <SendKnob key={r.id} track={track} ret={r} send={sends.find((s) => s.to === r.id)} sender={sender} />
        ))}
      </div>

      <Knob
        className="eth-strip__pan"
        size={28}
        bipolar
        value={(pan + 1) / 2}
        label={`${track.name} pan`}
        valueText={formatPan(pan)}
        onChange={(n) => void sender.send(cmd("Mixer", { type: "SetPan", track: track.id, pan: n * 2 - 1 }))}
        onChangeStart={sender.begin}
        onChangeEnd={sender.end}
      />

      <div className="eth-strip__fader-row">
        <Fader
          height={FADER_HEIGHT}
          value={dbToFader(volume)}
          defaultValue={dbToFader(0)}
          label={`${track.name} volume`}
          valueText={formatDb(volume)}
          onChange={(n) => void sender.send(cmd("Mixer", { type: "SetVolume", track: track.id, volume: faderToDb(n) }))}
          onChangeStart={sender.begin}
          onChangeEnd={sender.end}
        />
        <StripMeter track={track} />
      </div>
      <div className="eth-strip__db">{formatDb(volume)}</div>

      <div className="eth-strip__buttons">
        <Button
          size="sm"
          className="eth-strip__mute"
          active={mute}
          aria-label={`Mute ${track.name}`}
          onClick={() => void sender.send(cmd("Mixer", { type: "SetMute", track: track.id, mute: !mute }))}
        >
          M
        </Button>
        {!isMaster && (
          <Button
            size="sm"
            className="eth-strip__solo"
            active={solo}
            aria-label={`Solo ${track.name}`}
            title="Solo (Ctrl/Cmd-click to add to the soloed tracks)"
            onClick={(e) =>
              void sender.send(
                // Soloing is exclusive unless Ctrl/Cmd is held; unsoloing only affects this track.
                cmd("Mixer", {
                  type: "SetSolo",
                  track: track.id,
                  solo: !solo,
                  exclusive: !solo && !(e.ctrlKey || e.metaKey),
                }),
              )
            }
          >
            S
          </Button>
        )}
      </div>
    </div>
  );
}
