import clsx from "clsx";
import { Headphones, Music, Pencil, Volume2, VolumeX, Waves } from "lucide-react";
import { useRef, useState, type ReactNode } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Clip, Command, Track, TrackId, TrackSend } from "@/generated";
import { DeviceChain } from "@/features/devices";
import { useGestureSender, useSend, type GestureSender } from "@/features/devices/gesture";
import { formatDb, formatPan } from "@/features/devices/paramScale";
import { midiTarget } from "@/features/midi-learn/targets";
import { reverseCommand } from "@/features/clip-editing/clipEditing";
import { dbToFader, defaultOutputLabel, faderToDb, outputTargets, outputValue, parseOutputValue } from "@/features/mixer/routing";
import { GroupsRoutingRows } from "@/features/groups";
import { Button, IconButton, Knob, NumberField, Select, TextInput, Toggle } from "@/kit";
import { useEditorStore, useProjectStore } from "@/state";
import { TRACK_COLORS } from "@/theme";
import { formatBarBeat, formatDuration, useTempoMap } from "@/timeline";
import { cmd, newId } from "@/transport";
import type { InspectorTarget } from "./inspectorTarget";
import { useShellStore } from "./shellStore";
import "./inspector.css";

const PALETTE: ReadonlyArray<number> = TRACK_COLORS.map((c) => parseInt(c.slice(1), 16));
const css = (color: number) => `#${(color & 0xffffff).toString(16).padStart(6, "0")}`;
const batch = (label: string, commands: Command[]): Command | null =>
  commands.length === 0 ? null : commands.length === 1 ? commands[0]! : cmd("Edit", { type: "Batch", label, commands });

/**
 * Right-hand inspector: the selected clip(s), or the selected track, with its devices as a
 * stack of cards. While its pane plays the exit animation (nothing selected any more), it
 * keeps showing the last subject.
 */
export function Inspector({ target }: { target: InspectorTarget | null }) {
  const key = target ? JSON.stringify(target) : null;
  const [shown, setShown] = useState<{ key: string; target: InspectorTarget } | null>(null);
  if (target && key !== shown?.key) setShown({ key: key!, target });
  const t = target ?? shown?.target ?? null;
  if (!t) return null;
  return (
    <div className="eth-inspector" data-testid="inspector">
      {t.kind === "track" ? <TrackInspector id={t.id} /> : <ClipsInspector ids={t.ids} />}
    </div>
  );
}

// ---- Building blocks -----------------------------------------------------------------------

function Section({ title, children, className }: { title?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={clsx("eth-inspector__section", className)}>
      {title && <h3 className="eth-inspector__title">{title}</h3>}
      {children}
    </section>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="eth-inspector__row">
      <span className="eth-inspector__label">{label}</span>
      <div className="eth-inspector__control">{children}</div>
    </div>
  );
}

/** A name field committed on Enter / blur (Escape reverts). */
function NameField({ value, label, onCommit }: { value: string; label: string; onCommit(name: string): void }) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null && draft.trim() && draft !== value) onCommit(draft.trim());
    setDraft(null);
  };
  return (
    <TextInput
      size="sm"
      className="eth-inspector__name"
      aria-label={label}
      value={draft ?? value}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        else if (e.key === "Escape") {
          setDraft(null);
          (e.target as HTMLInputElement).blur();
        }
        e.stopPropagation();
      }}
    />
  );
}

/** Color swatches; `inherit` adds a "follow the track color" swatch (value null). */
function Swatches({
  value,
  onPick,
  inherit,
  label,
}: {
  value: number | null;
  onPick(color: number | null): void;
  inherit?: number;
  label: string;
}) {
  return (
    <div className="eth-inspector__swatches" role="radiogroup" aria-label={label}>
      {inherit !== undefined && (
        <button
          type="button"
          role="radio"
          aria-checked={value === null}
          aria-label="Track color"
          title="Use the track color"
          className="eth-inspector__swatch eth-inspector__swatch--inherit"
          style={{ ["--swatch" as string]: css(inherit) }}
          onClick={() => onPick(null)}
        />
      )}
      {PALETTE.map((c) => (
        <button
          key={c}
          type="button"
          role="radio"
          aria-checked={value === c}
          aria-label={css(c)}
          className="eth-inspector__swatch"
          style={{ ["--swatch" as string]: css(c) }}
          onClick={() => onPick(c)}
        />
      ))}
    </div>
  );
}

// ---- Clips ---------------------------------------------------------------------------------

function ClipsInspector({ ids }: { ids: ReadonlyArray<string> }) {
  const clips = useProjectStore(useShallow((s) => ids.flatMap((id) => (s.project?.clips[id] ? [s.project.clips[id]] : []))));
  if (clips.length === 0) return null;
  if (clips.length === 1) return <ClipInspector clip={clips[0]!} />;
  return <ManyClipsInspector clips={clips} />;
}

function ManyClipsInspector({ clips }: { clips: ReadonlyArray<Clip> }) {
  const send = useSend();
  const allMuted = clips.every((c) => c.muted);
  return (
    <>
      <Section>
        <div className="eth-inspector__heading">{clips.length} clips</div>
      </Section>
      <Section title="Clips">
        <Row label="Muted">
          <Toggle
            size="sm"
            aria-label="Mute clips"
            checked={allMuted}
            onChange={(v) => void send(cmd("Clip", { type: "SetMuted", ids: clips.map((c) => c.id), muted: v }))}
          />
        </Row>
        <Row label="Color">
          <Swatches
            label="Clip color"
            value={clips.every((c) => c.color === clips[0]!.color) ? clips[0]!.color : -1}
            inherit={0x888888}
            onPick={(color) => {
              const b = batch("Clip Color", clips.map((c) => cmd("Clip", { type: "SetColor", id: c.id, color })));
              if (b) void send(b);
            }}
          />
        </Row>
      </Section>
    </>
  );
}

function ClipInspector({ clip }: { clip: Clip }) {
  const send = useSend();
  // Number fields: a drag is one undo step (the fields report its start and end).
  const gesture = useGestureSender();
  const drag = { onChangeStart: gesture.begin, onChangeEnd: gesture.end };
  const tempo = useTempoMap();
  const track = useProjectStore((s) => s.project?.tracks[clip.track]);
  const audio = clip.content.type === "Audio" ? clip.content : null;
  const openEditor = () => {
    useShellStore.getState().openDrawer(audio ? "warp" : "piano-roll");
    useEditorStore.getState().openClip(clip.id);
  };

  return (
    <>
      <Section className="eth-inspector__head">
        <span className="eth-inspector__kind" style={{ ["--swatch" as string]: css(clip.color ?? track?.color ?? 0x888888) }}>
          {audio ? <Waves aria-hidden /> : <Music aria-hidden />}
        </span>
        <NameField
          label="Clip name"
          value={clip.name}
          onCommit={(name) => void send(cmd("Clip", { type: "Rename", id: clip.id, name }))}
        />
      </Section>

      <Section title="Clip">
        <Row label="Start">
          <span className="eth-inspector__value">{formatBarBeat(clip.start, tempo)}</span>
        </Row>
        <Row label="Length">
          <span className="eth-inspector__value">{formatDuration(clip.length)}</span>
        </Row>
        <Row label="Muted">
          <Toggle
            size="sm"
            aria-label="Mute clip"
            checked={clip.muted}
            onChange={(v) => void send(cmd("Clip", { type: "SetMuted", ids: [clip.id], muted: v }))}
          />
        </Row>
        <Row label="Loop">
          <Toggle
            size="sm"
            aria-label="Loop clip"
            checked={clip.looping.enabled}
            onChange={(v) => void send(cmd("Clip", { type: "SetLoop", id: clip.id, looping: { ...clip.looping, enabled: v } }))}
          />
        </Row>
        <Row label="Color">
          <Swatches
            label="Clip color"
            value={clip.color}
            inherit={track?.color ?? 0x888888}
            onPick={(color) => void send(cmd("Clip", { type: "SetColor", id: clip.id, color }))}
          />
        </Row>
        <div className="eth-inspector__actions">
          <Button size="sm" onClick={openEditor}>
            <Pencil aria-hidden /> {audio ? "Edit warp" : "Edit notes"}
          </Button>
        </div>
      </Section>

      {audio && (
        <Section title="Audio">
          <Row label="Gain">
            <NumberField
              {...drag}
              size="sm"
              aria-label="Clip gain"
              value={audio.gain}
              min={-60}
              max={24}
              step={0.5}
              precision={1}
              unit="dB"
              onChange={(gain) => void gesture.send(cmd("Clip", { type: "SetGain", id: clip.id, gain }))}
            />
          </Row>
          <Row label="Transpose">
            <NumberField
              {...drag}
              size="sm"
              aria-label="Clip transpose"
              value={audio.transpose}
              min={-48}
              max={48}
              unit="st"
              onChange={(semitones) => void gesture.send(cmd("Clip", { type: "SetTranspose", id: clip.id, semitones }))}
            />
          </Row>
          <Row label="Fade in">
            <NumberField
              {...drag}
              size="sm"
              aria-label="Fade in"
              value={audio.fade_in}
              min={0}
              max={clip.length}
              step={0.25}
              precision={2}
              unit="beats"
              onChange={(fade_in) => void gesture.send(cmd("Clip", { type: "SetFades", id: clip.id, fade_in, fade_out: audio.fade_out }))}
            />
          </Row>
          <Row label="Fade out">
            <NumberField
              {...drag}
              size="sm"
              aria-label="Fade out"
              value={audio.fade_out}
              min={0}
              max={clip.length}
              step={0.25}
              precision={2}
              unit="beats"
              onChange={(fade_out) => void gesture.send(cmd("Clip", { type: "SetFades", id: clip.id, fade_in: audio.fade_in, fade_out }))}
            />
          </Row>
          <Row label="Reverse">
            <Toggle
              size="sm"
              aria-label="Reverse clip"
              checked={audio.reversed}
              onChange={(v) => {
                const c = reverseCommand([clip], v);
                if (c) void send(c);
              }}
            />
          </Row>
          <Row label="Warp">
            <Toggle
              size="sm"
              aria-label="Warp clip"
              checked={audio.warp.enabled}
              onChange={(enabled) => void send(cmd("Warp", { type: "SetWarp", clip: clip.id, warp: { ...audio.warp, enabled } }))}
            />
          </Row>
          <Row label="Warp mode">
            <Select
              size="sm"
              aria-label="Warp mode"
              value={audio.warp.mode}
              disabled={!audio.warp.enabled}
              onChange={(mode) => void send(cmd("Warp", { type: "SetWarp", clip: clip.id, warp: { ...audio.warp, mode } }))}
              options={[
                { value: "Repitch", label: "Repitch" },
                { value: "Complex", label: "Complex" },
              ]}
            />
          </Row>
        </Section>
      )}

      {track && (
        <Section title={`Track · ${track.name}`}>
          <DeviceChain track={track.id} layout="stack" />
        </Section>
      )}
    </>
  );
}

// ---- Track ---------------------------------------------------------------------------------

function TrackInspector({ id }: { id: TrackId }) {
  const send = useSend();
  const sender = useGestureSender();
  const track = useProjectStore((s) => s.project?.tracks[id]);
  if (!track) return null;
  const isMaster = track.kind === "Master";

  return (
    <>
      <Section className="eth-inspector__head">
        <span className="eth-inspector__kind" style={{ ["--swatch" as string]: css(track.color) }} />
        {isMaster ? (
          <span className="eth-inspector__heading">{track.name}</span>
        ) : (
          <NameField
            label="Track name"
            value={track.name}
            onCommit={(name) => void send(cmd("Track", { type: "Rename", id: track.id, name }))}
          />
        )}
      </Section>

      {!isMaster && (
        <Section title="Color">
          <Swatches
            label="Track color"
            value={track.color}
            onPick={(color) => color !== null && void send(cmd("Track", { type: "SetColor", id: track.id, color }))}
          />
        </Section>
      )}

      <Section title="Mixer">
        <TrackMixer track={track} sender={sender} />
      </Section>

      <Section title="Devices">
        <DeviceChain track={track.id} layout="stack" />
      </Section>
    </>
  );
}

function TrackMixer({ track, sender }: { track: Track; sender: GestureSender }) {
  const send = useSend();
  const { volume, pan, mute, solo } = track.mixer;
  const isMaster = track.kind === "Master";
  // groups-buses: a VCA carries no audio (no pan, output or sends).
  const isVca = track.kind === "Vca";
  const returns = useProjectStore(
    useShallow((s) => (s.project ? Object.values(s.project.tracks).filter((t) => t.kind === "Return" && t.id !== track.id) : [])),
  );
  const sends = useProjectStore(
    useShallow((s) => (s.project ? Object.values(s.project.sends).filter((x) => x.from === track.id) : [])),
  );
  const targets = useProjectStore(useShallow((s) => (s.project ? outputTargets(s.project.tracks, track) : [])));
  const defaultLabel = useProjectStore((s) => (s.project ? defaultOutputLabel(s.project.tracks, track) : "Master"));

  return (
    <div className="eth-inspector__mixer">
      <div className="eth-inspector__knobs">
        <Knob
          size="lg"
          {...midiTarget({ type: "Param", target: { type: "TrackVolume", track: track.id } })}
          value={dbToFader(volume)}
          defaultValue={dbToFader(0)}
          label={`${track.name} volume`}
          valueText={formatDb(volume)}
          onChange={(n) => void sender.send(cmd("Mixer", { type: "SetVolume", track: track.id, volume: faderToDb(n) }))}
          onChangeStart={sender.begin}
          onChangeEnd={sender.end}
        />
        {!isVca && <Knob
          size="lg"
          bipolar
          {...midiTarget({ type: "Param", target: { type: "TrackPan", track: track.id } })}
          value={(pan + 1) / 2}
          label={`${track.name} pan`}
          valueText={formatPan(pan)}
          onChange={(n) => void sender.send(cmd("Mixer", { type: "SetPan", track: track.id, pan: n * 2 - 1 }))}
          onChangeStart={sender.begin}
          onChangeEnd={sender.end}
        />}
        <div className="eth-inspector__toggles">
          <IconButton
            size="sm"
            tone="ghost"
            active={mute}
            className="eth-inspector__mute"
            {...midiTarget({ type: "TrackMute", track: track.id })}
            label={`Mute ${track.name}`}
            icon={mute ? <VolumeX /> : <Volume2 />}
            onClick={() => void send(cmd("Mixer", { type: "SetMute", track: track.id, mute: !mute }))}
          />
          {!isMaster && (
            <IconButton
              size="sm"
              tone="ghost"
              active={solo}
              className="eth-inspector__solo"
              {...midiTarget({ type: "TrackSolo", track: track.id })}
              label={`Solo ${track.name}`}
              icon={<Headphones />}
              onClick={(e) =>
                void send(
                  cmd("Mixer", { type: "SetSolo", track: track.id, solo: !solo, exclusive: !solo && !(e.ctrlKey || e.metaKey) }),
                )
              }
            />
          )}
        </div>
      </div>

      {!isMaster && !isVca && (
        <Row label="Output">
          <Select
            size="sm"
            aria-label={`${track.name} output`}
            value={outputValue(track.output)}
            onChange={(v) => void send(cmd("Mixer", { type: "SetOutput", track: track.id, output: parseOutputValue(v) }))}
            options={[
              { value: "default", label: defaultLabel },
              ...targets.map((t) => ({ value: `track:${t.id}`, label: t.name })),
              { value: "none", label: "No output" },
            ]}
          />
        </Row>
      )}

      <GroupsRoutingRows track={track} Row={Row} />

      {!isMaster && !isVca && returns.length > 0 && (
        <div className="eth-inspector__sends" aria-label="Sends">
          {returns.map((r) => (
            <SendKnob key={r.id} track={track} ret={r} send={sends.find((x) => x.to === r.id)} sender={sender} />
          ))}
        </div>
      )}
    </div>
  );
}

/** A send to a return track; the first turn creates it (once, even mid-drag). */
function SendKnob({ track, ret, send, sender }: { track: Track; ret: Track; send: TrackSend | undefined; sender: GestureSender }) {
  const pending = useRef<string | null>(null);
  const level = send?.level ?? -144;
  return (
    <Knob
      size="sm"
      {...(send ? midiTarget({ type: "Param", target: { type: "SendLevel", send: send.id } }) : {})}
      value={send ? dbToFader(level) : 0}
      defaultValue={dbToFader(-144)}
      label={`Send ${ret.name}`}
      valueText={formatDb(level)}
      onChange={(n) => {
        const db = faderToDb(n);
        const id = send?.id ?? pending.current;
        if (id) {
          void sender.send(cmd("Mixer", { type: "SetSendLevel", send: id, level: db }));
          return;
        }
        const created = newId();
        if (sender.dragging()) pending.current = created;
        void sender.send(cmd("Mixer", { type: "CreateSend", id: created, from: track.id, to: ret.id, level: db, pre_fader: false }));
      }}
      onChangeStart={sender.begin}
      onChangeEnd={() => {
        pending.current = null;
        sender.end();
      }}
    />
  );
}
