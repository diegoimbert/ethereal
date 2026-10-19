/**
 * MIDI file import (#717): a `.mid` dropped from the browser or the OS onto the arrangement.
 *
 * The engine reads the file (`Media::ReadMidiFile`: a library file, an upload or a desktop
 * path; notes in beats, so they follow the file's tempo map musically). The UI turns it into
 * tracks and clips with ordinary commands, all in ONE `Edit::Batch` (one undo step):
 *
 * - Dropped on a MIDI track: the file's first track becomes a clip there; each other file
 *   track gets a new MIDI track (with the built-in synth) right after it.
 * - Dropped below the tracks (or with no target): one new MIDI track per file track.
 * - One clip per file track, all as long as the file (rounded up to whole bars), at the drop
 *   point. A type 0 file (one track, several channels) is split by channel by the engine.
 * - In a project with no clips yet, the file's tempo and time signature (and their
 *   changes) become the project's, so it plays at the tempo it was written at. Otherwise
 *   the project keeps its tempo and the notes land on its grid.
 */
import type { Beats, ClipId, Command, MediaSource, MidiFileContents, Project, TimeSignature, Track, TrackId } from "@/generated";
import { asOneStep } from "@/features/arrangement/editMath";
import { useArrangementUi, type PendingImport } from "@/features/arrangement/uiStore";
import type { ImportSource } from "@/features/import/sources";
import { withUpload } from "@/features/remote/upload";
import { useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, isCommandFailed, newId, type EngineTransport } from "@/transport";
import { t } from "@/i18n";

/** MIDI file extensions (`ether_controller::store::MIDI_EXTENSIONS`). */
export const MIDI_EXTENSIONS = ["mid", "midi"] as const;

export function isMidiName(name: string): boolean {
  const dot = name.lastIndexOf(".");
  return dot >= 0 && (MIDI_EXTENSIONS as readonly string[]).includes(name.slice(dot + 1).toLowerCase());
}

/** Where a file is read from: a browser item (engine-side source) or an OS file / path. */
export type MidiSource = { kind: "media"; source: MediaSource; name: string } | ImportSource;

export interface MidiTarget {
  /** A MIDI track to put the first file track on; `null`: new tracks only. */
  track: TrackId | null;
  at: Beats;
}

/** Read a MIDI file through the engine (an OS file is uploaded first). */
export async function readMidiFile(transport: EngineTransport, src: MidiSource): Promise<MidiFileContents> {
  const read = async (source: MediaSource) => {
    const reply = await transport.send(cmd("Media", { type: "ReadMidiFile", source }));
    if (reply.type !== "MidiFile") throw new Error(`unexpected reply ${reply.type}`);
    return reply.file;
  };
  if (src.kind === "media") return read(src.source);
  if (src.kind === "path") return read({ type: "Path", path: src.path });
  return withUpload(transport, src.file, undefined, {}, (upload) => read({ type: "Upload", upload }));
}

function signatureAt(project: Project, at: number): TimeSignature {
  const points = Object.values(project.time_signatures)
    .filter((s) => s.time <= at)
    .sort((a, b) => a.time - b.time);
  return points.at(-1)?.signature ?? { numerator: 4, denominator: 4 };
}

/** The next sibling of `track` that a regular track can be put before (`null`: the end). */
function nextSibling(project: Project, track: Track): TrackId | null {
  const siblings = Object.values(project.tracks)
    .filter((t) => t.parent === track.parent)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1));
  const next = siblings[siblings.indexOf(track) + 1];
  return next && next.kind !== "Return" && next.kind !== "Master" ? next.id : null;
}

export interface MidiImportPlan {
  command: Command;
  clips: ClipId[];
  tracks: TrackId[];
}

/**
 * The one-step command importing `file` at `target` (see the module docs). `null` when the
 * file has no notes. `id` makes the new ids (tests pass a counter).
 */
export function midiImportCommand(project: Project, file: MidiFileContents, target: MidiTarget, id: () => string = newId): MidiImportPlan | null {
  const tracks = file.tracks.filter((t) => t.notes.length > 0);
  if (tracks.length === 0) return null;
  const commands: Command[] = [];
  const adopt = Object.keys(project.clips).length === 0;
  if (adopt) {
    const tempo0 = Object.values(project.tempo_points).find((p) => p.time === 0);
    const first = file.tempos[0];
    if (tempo0 && first && first.time === 0) commands.push(cmd("Tempo", { type: "EditTempoPoint", id: tempo0.id, time: null, bpm: first.bpm, curve: null }));
    for (const t of file.tempos.filter((t) => t.time > 0)) {
      commands.push(cmd("Tempo", { type: "AddTempoPoint", id: id(), time: target.at + t.time, bpm: t.bpm, curve: "Step" }));
    }
    const sig0 = Object.values(project.time_signatures).find((s) => s.time === 0);
    const firstSig = file.time_signatures[0];
    if (sig0 && firstSig && firstSig.time === 0)
      commands.push(cmd("Tempo", { type: "EditTimeSignature", id: sig0.id, time: null, signature: firstSig.signature }));
    for (const s of file.time_signatures.filter((s) => s.time > 0)) {
      commands.push(cmd("Tempo", { type: "AddTimeSignature", id: id(), time: target.at + s.time, signature: s.signature }));
    }
  }
  const sig = adopt && file.time_signatures[0]?.time === 0 ? file.time_signatures[0].signature : signatureAt(project, target.at);
  const bar = (sig.numerator * 4) / sig.denominator;
  const length = Math.max(bar, Math.ceil(file.length / bar - 1e-9) * bar);

  const onTrack = target.track ? project.tracks[target.track] : undefined;
  const intoExisting = onTrack?.kind === "Midi" ? onTrack : undefined;
  const before = intoExisting ? nextSibling(project, intoExisting) : null;
  const parent = intoExisting ? intoExisting.parent : null;
  const several = tracks.length > 1;
  const clips: ClipId[] = [];
  const created: TrackId[] = [];
  tracks.forEach((t, i) => {
    const name = t.name ?? (several ? `${file.name} ${t.channel + 1}` : file.name);
    let track: TrackId;
    if (i === 0 && intoExisting) {
      track = intoExisting.id;
    } else {
      track = id();
      created.push(track);
      commands.push(cmd("Track", { type: "Create", id: track, kind: "Midi", name, color: null, parent, before }));
      commands.push(cmd("Device", { type: "Insert", id: id(), track, device: { type: "Builtin", device: { type: "Synth" } }, before: null }));
    }
    const clip = id();
    clips.push(clip);
    commands.push(cmd("Clip", { type: "CreateMidi", id: clip, track, start: target.at, length, name }));
    commands.push(
      cmd("Note", {
        type: "Add",
        clip,
        notes: t.notes.map((n) => ({ id: id(), pitch: n.pitch, velocity: n.velocity, start: n.start, duration: n.duration })),
      }),
    );
  });
  return { command: asOneStep("import midi file", commands)!, clips, tracks: created };
}

function message(e: unknown): string {
  if (isCommandFailed(e)) return e.code === "Decode" ? t("midi_file.not_readable") : e.error.message;
  return e instanceof Error ? e.message : String(e);
}

/** How long a failed import's lane placeholder stays up. */
export const MIDI_IMPORT_ERROR_MS = 4000;

/**
 * Read `src` and add it at `target` (one undo step), with a placeholder in the lane while it
 * runs; the new clips are selected. Resolves with them; rejects (after showing the error in
 * the placeholder) when the file can't be read or placed.
 */
export async function importMidiFile(transport: EngineTransport, src: MidiSource, target: MidiTarget): Promise<ClipId[]> {
  const ui = useArrangementUi.getState;
  const name = src.kind === "media" ? src.name : src.kind === "path" ? (src.path.split(/[\\/]/).pop() ?? src.path) : src.file.name;
  const pending: PendingImport = { id: newId(), track: target.track, at: target.at, name, progress: null, error: null };
  ui().putImport(pending);
  try {
    const file = await readMidiFile(transport, src);
    const project = useProjectStore.getState().project;
    if (!project) throw new Error("no project is open");
    const plan = midiImportCommand(project, file, target);
    if (!plan) throw new Error(`${name} has no notes`);
    await transport.send(plan.command);
    ui().removeImport(pending.id);
    itemSelection.getState().select("clip", plan.clips, "replace");
    return plan.clips;
  } catch (e) {
    ui().putImport({ ...pending, error: message(e) });
    setTimeout(() => ui().removeImport(pending.id), MIDI_IMPORT_ERROR_MS);
    throw e;
  }
}
