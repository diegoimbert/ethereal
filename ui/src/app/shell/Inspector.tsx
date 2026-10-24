import clsx from "clsx";
import { Headphones, Music, Volume2, VolumeX, Waves } from "lucide-react";
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Clip, Command, ParamInfo, Track, TrackId, TrackSend } from "@/generated";
import { inertTitle, useFrozenAncestor } from "@/features/freeze";
import { DeviceChain } from "@/features/devices";
import { collapseAll } from "@/features/devices/collapsed";
import { useGestureSender, useSend, type GestureSender } from "@/features/devices/gesture";
import { formatDb, formatPan, paramToPlain } from "@/domain/paramScale";
import { useLiveAutomation } from "@/features/automation/live";
import { CLIP_GAIN_INFO, CLIP_TRANSPOSE_INFO, PAN_INFO, SEND_INFO, VOLUME_INFO } from "@/features/automation/params";
import { openValueMenu } from "@/features/automation/valueMenu";
import { midiTarget } from "@/features/midi-learn/targets";
import { reverseCommand } from "@/features/clip-editing/clipEditing";
import {
  audioClips,
  common,
  gainBases,
  offsetGain,
  offsetTranspose,
  setAllGain,
  setAllTranspose,
  setAllWarpChoice,
  transposeBases,
  WARP_CHOICES,
  warpChoiceOf,
  warpChoicePatch,
  type AudioClip,
  type Bases,
  type WarpChoice,
} from "@/features/clip-editing/multiEdit";
import { dbToFader, faderToDb } from "@/features/arrangement/faderLaw";
import { defaultOutputLabel, outputTargets, outputValue, parseOutputValue } from "./outputRouting";
import { GroupsRoutingRows } from "@/features/groups";
import { FormantFields } from "@/features/warp/FormantFields";
import { MpeSettingsFields } from "@/features/mpe";
import { IconButton, Knob, NumberField, Popover, Select, TextInput, Toggle } from "@/kit";
import { useProjectStore, defaultClipName, defaultTrackName, tracksOrdered } from "@/state";
import { trackSoloCommand } from "@/domain/soloCommands";
import { additiveClick, muteOverriddenBySolo } from "@/domain/soloPlan";
import { ColorSwatches, colorSwatchCss } from "@/features/colors";
import { formatBarBeat, formatDuration, useTempoMap } from "@/timeline";
import { cmd, newId } from "@/transport";
import { type MessageKey, useT } from "@/i18n";
import type { InspectorTarget } from "./inspectorTarget";
import "./inspector.css";
import { ProjectSwingFields } from "@/features/groove";

const css = colorSwatchCss;
/** A clip's gain field (its range, -60..+24 dB) for the value menu's "set value". */
const GAIN_FIELD_INFO: ParamInfo = { ...CLIP_GAIN_INFO, name: "gain", min: -60, max: 24 };
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

function Row({ label, title, children }: { label: string; title?: string; children: ReactNode }) {
  return (
    <div className="eth-inspector__row" title={title}>
      <span className="eth-inspector__label">{label}</span>
      <div className="eth-inspector__control">{children}</div>
    </div>
  );
}

/** A name field committed on Enter / blur (Escape reverts). */
/** What an unnamed track / clip is called in its name field (message keys). */
const TRACK_KIND_NAME: Record<Track["kind"], MessageKey> = {
  Midi: "shell.inspector.kind_midi",
  Audio: "shell.inspector.kind_audio",
  Group: "shell.inspector.kind_group",
  Return: "shell.inspector.kind_return",
  Vca: "shell.inspector.kind_vca",
  Master: "shell.inspector.kind_master",
};

function NameField({
  value,
  label,
  placeholder,
  onCommit,
  defaultName,
}: {
  value: string;
  label: string;
  placeholder?: string;
  onCommit(name: string): void;
  /** The name an emptied field goes back to (the entity's default name). */
  defaultName?(): string;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null) {
      const next = draft.trim() || (defaultName?.() ?? "");
      if ((next || defaultName) && next !== value) onCommit(next);
    }
    setDraft(null);
  };
  return (
    <TextInput
      size="lg"
      // A name, and its placeholder may be one too (an unnamed clip reads as its track's).
      className="eth-inspector__name eth-user-text"
      aria-label={label}
      placeholder={placeholder}
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

/**
 * The current color as one swatch; clicking it opens the palette in a popover. `kind`: the
 * swatch is the header's kind square (the clip's / track's icon on its color) instead.
 */
function ColorPicker({
  value,
  onPick,
  inherit,
  label,
  kind,
}: {
  value: number | null;
  onPick(color: number | null): void;
  inherit?: number | null;
  label: string;
  kind?: ReactNode;
}) {
  const shown = value === null || value < 0 ? inherit : value;
  return (
    <Popover
      aria-label={label}
      placement="bottom-start"
      trigger={(p) => (
        <button
          type="button"
          className={clsx(
            kind === undefined ? "eth-color-swatch eth-inspector__color" : "eth-inspector__kind eth-inspector__kind--pick",
            kind === undefined && value === null && inherit !== undefined && "eth-color-swatch--inherit",
          )}
          style={{ ["--swatch" as string]: css(shown) }}
          aria-label={label}
          title={label}
          {...p}
        >
          {kind}
        </button>
      )}
    >
      {(close) => (
        <div className="eth-color-palette">
          <ColorSwatches
            label={label}
            value={value}
            inherit={inherit}
            onPick={(c) => {
              onPick(c);
              close();
            }}
          />
        </div>
      )}
    </Popover>
  );
}

/** A compact field: a small label over its control (several per row). */
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="eth-inspector__field">
      <span className="eth-inspector__field-label">{label}</span>
      {children}
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
  const t = useT();
  const send = useSend();
  const allMuted = clips.every((c) => c.muted);
  const kind = common(clips.map((c) => c.content.type));
  const heading =
    kind === "Audio" ? "shell.inspector.audio_clips_selected" : kind === "Midi" ? "shell.inspector.midi_clips_selected" : "shell.inspector.clips_selected";
  return (
    <>
      <Section>
        <div className="eth-inspector__heading" data-testid="inspector-clip-count">
          {t(heading, { count: clips.length })}
        </div>
      </Section>
      <Section title={t("shell.inspector.clips")}>
        <Row label={t("shell.inspector.muted")}>
          <Toggle
            size="sm"
            aria-label={t("shell.inspector.mute_clips")}
            checked={allMuted}
            onChange={(v) => void send(cmd("Clip", { type: "SetMuted", ids: clips.map((c) => c.id), muted: v }))}
          />
        </Row>
        <Row label={t("shell.inspector.color")}>
          <ColorPicker
            label={t("shell.inspector.clip_color")}
            value={clips.every((c) => c.color === clips[0]!.color) ? clips[0]!.color : -1}
            inherit={null}
            onPick={(color) => {
              const b = batch(
                "set clip color",
                clips.map((c) => cmd("Clip", { type: "SetColor", id: c.id, color })),
              );
              if (b) void send(b);
            }}
          />
        </Row>
      </Section>
      <ManyAudioClipsSection clips={clips} />
    </>
  );
}

/**
 * The audio fields of several clips: typing or picking a value sets it on every
 * selected audio clip; a drag or an arrow step moves each by the same amount (their
 * differences are kept). Differing values show as mixed ("—"). One undo step per edit
 * (an `Edit::Batch`; a drag is one gesture).
 */
function ManyAudioClipsSection({ clips }: { clips: ReadonlyArray<Clip> }) {
  const t = useT();
  const send = useSend();
  const gesture = useGestureSender();
  const audio = audioClips(clips);
  // Values when the current drag started (each clip moves from its own).
  const bases = useRef<{ transpose: Bases; gain: Bases } | null>(null);
  if (audio.length === 0) return null;
  const drag = {
    onChangeStart: () => {
      bases.current = { transpose: transposeBases(audio), gain: gainBases(audio) };
      gesture.begin();
    },
    onChangeEnd: () => {
      bases.current = null;
      gesture.end();
    },
  };
  const sendCmd = (c: Command | null) => {
    if (c) void gesture.send(c);
  };
  const gain = common(audio.map((a) => a.audio.gain));
  const transpose = common(audio.map((a) => a.audio.transpose));
  const reversed = audio.every((a) => a.audio.reversed);
  // The mode list, "none" (warp off) included: a mode the clips don't share shows as "—".
  const choice = common(audio.map((a) => warpChoiceOf(a.audio.warp)));
  const allClips = (list: ReadonlyArray<AudioClip>) => list.map((a) => a.clip);
  return (
    <Section title={audio.length === clips.length ? t("shell.inspector.audio") : t("shell.inspector.audio_some", { count: audio.length, total: clips.length })}>
      <Row label={t("shell.inspector.gain")}>
        <NumberField
          {...drag}
          size="sm"
          aria-label={t("shell.inspector.clip_gain")}
          value={gain ?? audio[0]!.audio.gain}
          mixed={gain === null}
          min={-60}
          max={24}
          step={0.5}
          precision={1}
          unit="db"
          onChange={(v) => sendCmd(setAllGain(audio, v))}
          onContextMenu={(e) => openValueMenu(e, { info: GAIN_FIELD_INFO, plain: gain ?? audio[0]!.audio.gain, set: (v) => sendCmd(setAllGain(audio, v)) })}
          onNudge={(d) => sendCmd(offsetGain(audio, bases.current?.gain ?? gainBases(audio), d))}
        />
      </Row>
      <Row label={t("shell.inspector.transpose")}>
        <NumberField
          {...drag}
          size="sm"
          aria-label={t("shell.inspector.clip_transpose")}
          value={transpose ?? audio[0]!.audio.transpose}
          mixed={transpose === null}
          min={-48}
          max={48}
          unit={t("shell.inspector.unit_semitones")}
          onChange={(v) => sendCmd(setAllTranspose(audio, v))}
          onContextMenu={(e) =>
            openValueMenu(e, { info: CLIP_TRANSPOSE_INFO, plain: transpose ?? audio[0]!.audio.transpose, set: (v) => sendCmd(setAllTranspose(audio, v)) })
          }
          onNudge={(d) => sendCmd(offsetTranspose(audio, bases.current?.transpose ?? transposeBases(audio), d))}
        />
      </Row>
      <Row label={t("shell.inspector.reverse")}>
        <Toggle
          size="sm"
          aria-label={t("shell.inspector.reverse_clips")}
          checked={reversed}
          onChange={(v) => {
            const c = reverseCommand(allClips(audio.filter((a) => a.audio.reversed !== v)), v);
            if (c) void send(c);
          }}
        />
      </Row>
      <Row label={t("shell.inspector.warp_mode")}>
        <WarpChoiceSelect
          value={choice}
          onChange={(c) => {
            const command = setAllWarpChoice(audio, c);
            if (command) void send(command);
          }}
        />
      </Row>
      {choice === "ComplexPlus" && <FormantFields targets={audio} row={formantRow} />}
    </Section>
  );
}

/** The warp/pitch mode list ("none" = warp off); `null`: the clips differ ("—"). */
function WarpChoiceSelect({ value, onChange }: { value: WarpChoice | null; onChange: (c: WarpChoice) => void }) {
  const t = useT();
  return (
    <Select<WarpChoice>
      size="sm"
      aria-label={t("shell.inspector.warp_mode")}
      value={(value ?? "") as WarpChoice}
      placeholder="—"
      onChange={onChange}
      options={WARP_CHOICES.map((m) => ({ value: m.value, label: t(m.label) }))}
    />
  );
}

function ClipInspector({ clip }: { clip: Clip }) {
  const t = useT();
  const send = useSend();
  // Number fields: a drag is one undo step (the fields report its start and end).
  const gesture = useGestureSender();
  const drag = { onChangeStart: gesture.begin, onChangeEnd: gesture.end };
  const tempo = useTempoMap();
  const track = useProjectStore((s) => s.project?.tracks[clip.track]);
  const audio = clip.content.type === "Audio" ? clip.content : null;

  return (
    <>
      <Section className="eth-inspector__head">
        {/* The kind square is the color picker. */}
        <ColorPicker
          label={t("shell.inspector.clip_color")}
          value={clip.color}
          inherit={track?.color ?? null}
          onPick={(color) => void send(cmd("Clip", { type: "SetColor", id: clip.id, color }))}
          kind={audio ? <Waves aria-hidden /> : <Music aria-hidden />}
        />
        <NameField
          label={t("shell.inspector.clip_name")}
          // Unnamed, it reads as its track.
          placeholder={track?.name || (audio ? t("shell.inspector.audio_clip") : t("shell.inspector.midi_clip"))}
          value={clip.name}
          onCommit={(name) => void send(cmd("Clip", { type: "Rename", id: clip.id, name }))}
          defaultName={() => {
            const project = useProjectStore.getState().project;
            return project ? defaultClipName(project, clip) : "";
          }}
        />
      </Section>

      {audio && (
        <Section title={t("shell.inspector.audio")}>
          <Row label={t("shell.inspector.gain")}>
            <NumberField
              {...drag}
              size="sm"
              aria-label={t("shell.inspector.clip_gain")}
              value={audio.gain}
              min={-60}
              max={24}
              step={0.5}
              precision={1}
              unit="db"
              onChange={(gain) => void gesture.send(cmd("Clip", { type: "SetGain", id: clip.id, gain }))}
              onContextMenu={(e) =>
                openValueMenu(e, { info: GAIN_FIELD_INFO, plain: audio.gain, set: (gain) => void send(cmd("Clip", { type: "SetGain", id: clip.id, gain })) })
              }
            />
          </Row>
          <Row label={t("shell.inspector.transpose")}>
            <NumberField
              {...drag}
              size="sm"
              aria-label={t("shell.inspector.clip_transpose")}
              value={audio.transpose}
              min={-48}
              max={48}
              unit={t("shell.inspector.unit_semitones")}
              onChange={(semitones) => void gesture.send(cmd("Clip", { type: "SetTranspose", id: clip.id, semitones }))}
              onContextMenu={(e) =>
                openValueMenu(e, {
                  info: CLIP_TRANSPOSE_INFO,
                  plain: audio.transpose,
                  set: (semitones) => void send(cmd("Clip", { type: "SetTranspose", id: clip.id, semitones })),
                })
              }
            />
          </Row>
          <Row label={t("shell.inspector.fade_in")}>
            <NumberField
              {...drag}
              size="sm"
              aria-label={t("shell.inspector.fade_in")}
              value={audio.fade_in}
              min={0}
              max={clip.length}
              step={0.25}
              precision={2}
              unit={t("shell.inspector.unit_beats")}
              onChange={(fade_in) => void gesture.send(cmd("Clip", { type: "SetFades", id: clip.id, fade_in, fade_out: audio.fade_out }))}
            />
          </Row>
          <Row label={t("shell.inspector.fade_out")}>
            <NumberField
              {...drag}
              size="sm"
              aria-label={t("shell.inspector.fade_out")}
              value={audio.fade_out}
              min={0}
              max={clip.length}
              step={0.25}
              precision={2}
              unit={t("shell.inspector.unit_beats")}
              onChange={(fade_out) => void gesture.send(cmd("Clip", { type: "SetFades", id: clip.id, fade_in: audio.fade_in, fade_out }))}
            />
          </Row>
          <Row label={t("shell.inspector.reverse")}>
            <Toggle
              size="sm"
              aria-label={t("shell.inspector.reverse_clip")}
              checked={audio.reversed}
              onChange={(v) => {
                const c = reverseCommand([clip], v);
                if (c) void send(c);
              }}
            />
          </Row>
          <Row label={t("shell.inspector.warp_mode")}>
            <WarpChoiceSelect
              value={warpChoiceOf(audio.warp)}
              onChange={(c) => void send(cmd("Warp", { type: "SetWarp", clip: clip.id, warp: { ...audio.warp, ...warpChoicePatch(c) } }))}
            />
          </Row>
          {warpChoiceOf(audio.warp) === "ComplexPlus" && <FormantFields targets={[{ clip, audio }]} row={formantRow} />}
        </Section>
      )}

      <Section>
        <div className="eth-inspector__fields">
          <Field label={t("shell.inspector.start")}>
            <span className="eth-inspector__value">{formatBarBeat(clip.start, tempo)}</span>
          </Field>
          <Field label={t("shell.inspector.length")}>
            <span className="eth-inspector__value">{formatDuration(clip.length)}</span>
          </Field>
          <Field label={t("shell.inspector.muted")}>
            <Toggle
              size="sm"
              aria-label={t("shell.inspector.mute_clip")}
              checked={clip.muted}
              onChange={(v) => void send(cmd("Clip", { type: "SetMuted", ids: [clip.id], muted: v }))}
            />
          </Field>
          <Field label={t("shell.inspector.loop")}>
            <Toggle
              size="sm"
              aria-label={t("shell.inspector.loop_clip")}
              checked={clip.looping.enabled}
              onChange={(v) => void send(cmd("Clip", { type: "SetLoop", id: clip.id, looping: { ...clip.looping, enabled: v } }))}
            />
          </Field>
          {/* The project's playback swing (every MIDI clip). */}
          {!audio && (
            <Field label={t("shell.inspector.groove")}>
              <ProjectSwingFields />
            </Field>
          )}
        </div>
      </Section>

      {track && (
        <Section>
          <DeviceChain track={track.id} layout="stack" />
        </Section>
      )}
    </>
  );
}

// ---- Track ---------------------------------------------------------------------------------

function TrackInspector({ id }: { id: TrackId }) {
  // Showing a track (opening the inspector, or another track) starts with its devices collapsed.
  useLayoutEffect(() => collapseAll(), [id]);
  const t = useT();
  const send = useSend();
  const sender = useGestureSender();
  const track = useProjectStore((s) => s.project?.tracks[id]);
  if (!track) return null;
  const isMaster = track.kind === "Master";

  return (
    <>
      <Section className="eth-inspector__head">
        {/* The kind square is the color picker (the master's color is fixed). */}
        {isMaster ? (
          <span className="eth-inspector__kind" style={{ ["--swatch" as string]: css(track.color) }} />
        ) : (
          <ColorPicker
            label={t("shell.inspector.track_color")}
            value={track.color}
            onPick={(color) => color !== null && void send(cmd("Track", { type: "SetColor", id: track.id, color }))}
            kind={null}
          />
        )}
        {isMaster ? (
          <span className="eth-inspector__heading eth-user-text">{track.name}</span>
        ) : (
          <NameField
            label={t("shell.inspector.track_name")}
            placeholder={t(TRACK_KIND_NAME[track.kind])}
            value={track.name}
            onCommit={(name) => void send(cmd("Track", { type: "Rename", id: track.id, name }))}
            defaultName={() => {
              const project = useProjectStore.getState().project;
              return project ? defaultTrackName(project, track) : track.name;
            }}
          />
        )}
      </Section>

      <Section title={t("shell.inspector.mixer")}>
        <TrackMixer track={track} sender={sender} />
      </Section>

      {track.kind === "Midi" && (
        <Section title="mpe">
          <MpeSettingsFields track={track} Row={Row} />
        </Section>
      )}

      {/* A VCA carries no audio, so it takes no devices (the engine rejects inserts). */}
      {track.kind !== "Vca" && (
        <Section>
          <DeviceChain track={track.id} layout="stack" />
        </Section>
      )}
    </>
  );
}

function TrackMixer({ track, sender }: { track: Track; sender: GestureSender }) {
  const t = useT();
  const send = useSend();
  const { mute, solo } = track.mixer;
  // Inside a frozen group (Freeze Group): mute/solo do nothing until the group is unfrozen.
  const inert = useFrozenAncestor(track.id);
  // Automated volume / pan: the knobs show what the engine plays at the playhead.
  const liveVolume = useLiveAutomation({ type: "TrackVolume", track: track.id });
  const livePan = useLiveAutomation({ type: "TrackPan", track: track.id });
  const volume = liveVolume === null ? track.mixer.volume : paramToPlain(VOLUME_INFO, liveVolume);
  const pan = livePan === null ? track.mixer.pan : paramToPlain(PAN_INFO, livePan);
  const isMaster = track.kind === "Master";
  // A VCA carries no audio (no pan, output or sends).
  const isVca = track.kind === "Vca";
  const returns = useProjectStore(
    // In the returns' order (the arrangement's): reordering the returns reorders the send knobs.
    useShallow((s) => (s.project ? tracksOrdered(s.project).filter((t) => t.kind === "Return" && t.id !== track.id) : [])),
  );
  const sends = useProjectStore(useShallow((s) => (s.project ? Object.values(s.project.sends).filter((x) => x.from === track.id) : [])));
  const targets = useProjectStore(useShallow((s) => (s.project ? outputTargets(s.project.tracks, track) : [])));
  const defaultLabel = useProjectStore((s) => (s.project ? defaultOutputLabel(s.project.tracks, track) : t("shell.inspector.output_master")));

  return (
    <div className="eth-inspector__mixer">
      <div className="eth-inspector__knobs">
        <Knob
          size="lg"
          {...midiTarget({ type: "Param", target: { type: "TrackVolume", track: track.id } })}
          value={dbToFader(volume)}
          defaultValue={dbToFader(0)}
          label={t("shell.inspector.volume_named", { name: track.name })}
          valueText={formatDb(volume)}
          onChange={(n) => void sender.send(cmd("Mixer", { type: "SetVolume", track: track.id, volume: faderToDb(n) }))}
          onContextMenu={(e) =>
            openValueMenu(e, {
              info: VOLUME_INFO,
              plain: volume,
              set: (v) => void send(cmd("Mixer", { type: "SetVolume", track: track.id, volume: v })),
              automation: { track: track.id, target: { type: "TrackVolume", track: track.id } },
            })
          }
          onChangeStart={sender.begin}
          onChangeEnd={sender.end}
        />
        {!isVca && (
          <Knob
            size="lg"
            bipolar
            {...midiTarget({ type: "Param", target: { type: "TrackPan", track: track.id } })}
            value={(pan + 1) / 2}
            label={t("shell.inspector.pan_named", { name: track.name })}
            valueText={formatPan(pan)}
            onChange={(n) => void sender.send(cmd("Mixer", { type: "SetPan", track: track.id, pan: n * 2 - 1 }))}
            onContextMenu={(e) =>
              openValueMenu(e, {
                info: PAN_INFO,
                plain: pan,
                set: (v) => void send(cmd("Mixer", { type: "SetPan", track: track.id, pan: v })),
                automation: { track: track.id, target: { type: "TrackPan", track: track.id } },
              })
            }
            onChangeStart={sender.begin}
            onChangeEnd={sender.end}
          />
        )}
        <div className="eth-inspector__toggles">
          <IconButton
            size="sm"
            tone="ghost"
            active={mute}
            className="eth-inspector__mute"
            data-solo-override={muteOverriddenBySolo(mute, solo) || undefined}
            {...midiTarget({ type: "TrackMute", track: track.id })}
            label={t("shell.inspector.mute_named", { name: track.name })}
            disabled={!!inert}
            title={inert ? inertTitle(inert) : muteOverriddenBySolo(mute, solo) ? t("mixer.mute_overridden") : undefined}
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
              label={t("shell.inspector.solo_named", { name: track.name })}
              disabled={!!inert}
              title={inert ? inertTitle(inert) : undefined}
              icon={<Headphones />}
              onClick={(e) => void send(trackSoloCommand(Object.values(useProjectStore.getState().project?.tracks ?? {}), track.id, additiveClick(e)))}
            />
          )}
        </div>
      </div>

      {!isMaster && !isVca && (
        <Row label={t("shell.inspector.output")}>
          <Select
            size="sm"
            aria-label={t("shell.inspector.output_named", { name: track.name })}
            value={outputValue(track.output)}
            onChange={(v) => void send(cmd("Mixer", { type: "SetOutput", track: track.id, output: parseOutputValue(v) }))}
            options={[
              { value: "default", label: defaultLabel },
              ...targets.map((x) => ({ value: `track:${x.id}`, label: x.name, userText: true })),
              { value: "none", label: t("shell.inspector.no_output") },
            ]}
          />
        </Row>
      )}

      <GroupsRoutingRows track={track} Row={Row} />

      {!isMaster && !isVca && returns.length > 0 && (
        <div className="eth-inspector__sends" aria-label={t("shell.inspector.sends")}>
          {returns.map((r) => (
            <SendKnob key={r.id} track={track} ret={r} send={sends.find((x) => x.to === r.id)} sender={sender} />
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * A send to a return track; the first turn creates it (once, even mid-drag). Under it, the
 * send's tap point: post-fader (the default, follows the track's volume) or pre-fader (a
 * cue mix that ignores the fader). One command, so one undo step; disabled until the send
 * exists.
 */
function SendKnob({ track, ret, send, sender }: { track: Track; ret: Track; send: TrackSend | undefined; sender: GestureSender }) {
  const t = useT();
  const pending = useRef<string | null>(null);
  const sendNow = useSend();
  const level = send?.level ?? -144;
  const pre = send?.pre_fader ?? false;
  return (
    <div className="eth-inspector__send">
      <Knob
        size="sm"
        {...(send ? midiTarget({ type: "Param", target: { type: "SendLevel", send: send.id } }) : {})}
        value={send ? dbToFader(level) : 0}
        defaultValue={dbToFader(-144)}
        label={t("shell.inspector.send_named", { name: ret.name })}
        valueText={formatDb(level)}
        // The value menu; a send not made yet is made by "set value".
        onContextMenu={(e) =>
          openValueMenu(e, {
            info: SEND_INFO,
            plain: level,
            set: (db) =>
              void sendNow(
                send
                  ? cmd("Mixer", { type: "SetSendLevel", send: send.id, level: db })
                  : cmd("Mixer", { type: "CreateSend", id: newId(), from: track.id, to: ret.id, level: db, pre_fader: false }),
              ),
            automation: send ? { track: track.id, target: { type: "SendLevel", send: send.id } } : undefined,
          })
        }
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
      <button
        type="button"
        className="eth-inspector__send-tap"
        aria-pressed={pre}
        aria-label={t("shell.inspector.send_pre_named", { name: ret.name })}
        title={send ? (pre ? t("shell.inspector.pre_title") : t("shell.inspector.post_title")) : t("shell.inspector.send_first")}
        disabled={!send}
        onClick={() => send && void sendNow(cmd("Mixer", { type: "SetSendPreFader", send: send.id, pre_fader: !pre }))}
      >
        {pre ? t("shell.inspector.pre") : t("shell.inspector.post")}
      </button>
    </div>
  );
}

/** An inspector row for one of Complex+'s formant fields (`FormantFields`). */
function formantRow(key: string, label: string, title: string, field: ReactNode) {
  return (
    <Row key={key} label={label} title={title}>
      {field}
    </Row>
  );
}
