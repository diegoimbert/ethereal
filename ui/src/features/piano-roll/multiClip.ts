/**
 * Multi-clip editing (base-143, owner request, Ableton Live 11 style): with several MIDI
 * clips selected in the arrangement, the piano roll shows all their notes on one pitch grid
 * on the ARRANGEMENT timeline (song beats), and edits them as if they were one clip.
 *
 * The time mapping (pure, unit-tested):
 *
 * - A note of clip `c` shows at song beat `content + shift(c)`, `shift = c.start - c.offset`
 *   (the clip's first pass). The mapping is a pure shift per clip, so an edit computed on the
 *   shown notes (moves, resizes, stretches, nudges) maps back by subtracting the note's clip
 *   shift; durations and offsets are unchanged.
 * - Each clip is a band `[c.start, c.start + c.length)` of the editor. Its EDITABLE window is
 *   the first pass: the whole band, or up to the loop end when the clip loops; the loop
 *   repetitions after it show the loop's notes as ghosts (not interactive).
 * - Notes shown are the ones starting inside their clip's editable window; a move keeps every
 *   note in its own clip and inside that window (clamped, like Ableton: notes never jump
 *   between clips). Drawing and pasting put a note in the clip under it, on the focused clip's
 *   track first (a press in a loop repetition writes into the loop); nothing is drawn outside
 *   every clip.
 *
 * Commands: the piano roll's components keep sending ordinary note commands in the editor's
 * axis; `multiClipTransport` (a transport wrapper) maps them to clip content before sending:
 * `Note.Edit` starts, and `Note.Quantize` / `Groove.Humanize` split per clip (in one
 * `Edit.Batch`). A gesture over notes of several clips is therefore one undo step, replicated
 * in collab like any document edit; no protocol change.
 */

import type { Beats, Clip, ClipId, Command, Note, NoteId, NoteSpec, Project, TextKey, TrackId } from "@/generated";
import type { StepLabel } from "@/features/arrangement/editMath";
import type { EngineTransport, SendOptions } from "@/transport";
import { cmd, newId } from "@/transport";
import { useProjectStore } from "@/state";
import { songToContent } from "./clipTime";
import { MIN_NOTE_BEATS } from "./noteEdits";
import type { NoteClipboard, PianoRollSection } from "./section";

const EPS = 1e-9;

/** Section scope key of the multi-clip editor (sections are scoped per editor). */
export const MULTI_SECTION_KEY: ClipId = "multi-clip";

/** Song beat − content beat for the notes of `clip` (its first pass). */
export const clipShift = (clip: Clip): Beats => clip.start - clip.offset;

/** A clip's place in the multi-clip editor (song beats). */
export interface ClipBand {
  clip: Clip;
  /** The clip's region on the timeline. */
  start: Beats;
  end: Beats;
  /** End of the editable first pass (`end`, or the first loop end when looping). */
  editEnd: Beats;
  /** The loop region of the first pass (clipped to the band), or null. */
  loop: { start: Beats; end: Beats } | null;
}

export function clipBand(clip: Clip): ClipBand {
  const start = clip.start;
  const end = clip.start + clip.length;
  const shift = clipShift(clip);
  const { enabled, start: ls, end: le } = clip.looping;
  if (!enabled || le - ls <= EPS) return { clip, start, end, editEnd: end, loop: null };
  const editEnd = Math.min(end, Math.max(start, shift + le));
  const loopStart = Math.max(start, shift + ls);
  return { clip, start, end, editEnd, loop: editEnd - loopStart > EPS ? { start: loopStart, end: editEnd } : null };
}

/** `note` on the song axis (its clip's shift added to its start). */
export const toSongNote = (note: Note, clip: Clip): Note => ({ ...note, start: note.start + clipShift(clip) });

/** Whether a song-axis start lies inside a band's editable window. */
export const inEditWindow = (band: ClipBand, start: Beats): boolean => start >= band.start - EPS && start < band.editEnd - EPS;

/**
 * The shown notes of every clip on the song axis (the ones starting in their clip's editable
 * window), sorted by start then pitch.
 */
export function songNotes(bands: ReadonlyArray<ClipBand>, notesOf: (clip: ClipId) => ReadonlyArray<Note>): Note[] {
  const out: Note[] = [];
  for (const b of bands) {
    for (const n of notesOf(b.clip.id)) {
      const s = toSongNote(n, b.clip);
      if (inEditWindow(b, s.start)) out.push(s);
    }
  }
  return out.sort((a, b) => a.start - b.start || a.pitch - b.pitch);
}

/** A non-interactive copy of a looped clip's note in a loop repetition (song axis). */
export interface GhostNote {
  key: string;
  note: Note;
}

/**
 * The loop repetitions of each looping clip: every note starting inside the loop region,
 * repeated after the first pass until the clip ends (song axis).
 */
export function loopGhosts(bands: ReadonlyArray<ClipBand>, notesOf: (clip: ClipId) => ReadonlyArray<Note>, limit = 2000): GhostNote[] {
  const out: GhostNote[] = [];
  for (const b of bands) {
    const { enabled, start: ls, end: le } = b.clip.looping;
    const len = le - ls;
    if (!enabled || len <= EPS || b.editEnd >= b.end - EPS) continue;
    const inLoop = notesOf(b.clip.id).filter((n) => n.start >= ls - EPS && n.start < le - EPS);
    if (!inLoop.length) continue;
    for (let k = 0, at = b.editEnd; at < b.end - EPS && out.length < limit; k++, at += len) {
      for (const n of inLoop) {
        const start = at + (n.start - ls);
        if (start >= b.end - EPS) continue;
        out.push({ key: `${n.id}:${k}`, note: { ...n, start, duration: Math.min(n.duration, b.end - start) } });
      }
    }
  }
  return out;
}

/** The parts of `[from, to)` outside every band (the shaded "no clip" areas). */
export function outsideBands(bands: ReadonlyArray<Pick<ClipBand, "start" | "end">>, from: Beats, to: Beats): Array<{ start: Beats; end: Beats }> {
  const sorted = [...bands].sort((a, b) => a.start - b.start);
  const out: Array<{ start: Beats; end: Beats }> = [];
  let at = from;
  for (const b of sorted) {
    if (b.start > at + EPS) out.push({ start: at, end: Math.min(b.start, to) });
    at = Math.max(at, b.end);
    if (at >= to) break;
  }
  if (at < to - EPS) out.push({ start: at, end: to });
  return out.filter((g) => g.end - g.start > EPS);
}

/**
 * The clip a note placed at song beat `at` lands in: the first band (in `tracks` order:
 * the preferred tracks) whose region contains `at`. Null outside every clip of those tracks.
 */
export function bandAt(bands: ReadonlyArray<ClipBand>, at: Beats, tracks: ReadonlyArray<TrackId>): ClipBand | null {
  for (const t of tracks) {
    const b = bands.find((x) => x.clip.track === t && at >= x.start - EPS && at < x.end - EPS);
    if (b) return b;
  }
  return null;
}

/** Content position of song beat `at` in `clip` (a loop repetition maps into the loop). */
export function contentAt(clip: Clip, at: Beats): Beats {
  return Math.max(0, songToContent(clip, at) ?? at - clipShift(clip));
}

/**
 * Move limits on the song axis for a note of the band: its start stays inside the band's
 * editable window (a note already outside it may stay where it is).
 */
export function bandLimits(band: ClipBand, note: Note): readonly [Beats, Beats] {
  const lo = Math.min(band.start, note.start);
  const hi = Math.max(band.editEnd - MIN_NOTE_BEATS, band.start, note.start);
  return [lo, hi];
}

/**
 * Paste `cb` at song beat `at` in the multi-clip editor: each note goes into the clip under it
 * on its source clip's track, else on `focusTrack` (notes over no clip are dropped: like
 * Ableton, nothing lives outside a clip). Clips never grow here. One undo step.
 */
export function multiPasteCommand(
  bands: ReadonlyArray<ClipBand>,
  cb: NoteClipboard,
  at: Beats,
  focusTrack: TrackId,
  label: StepLabel,
): { command: Command | null; ids: NoteId[]; section: PianoRollSection } {
  at = Math.max(0, at);
  const trackOf = new Map(bands.map((b) => [b.clip.id, b.clip.track]));
  const adds = new Map<ClipId, NoteSpec[]>();
  const muted: NoteId[] = [];
  const ids: NoteId[] = [];
  for (const n of cb.notes) {
    const s = at + n.start;
    const source = n.clip ? trackOf.get(n.clip) : undefined;
    const band = bandAt(bands, s, source ? [source, focusTrack] : [focusTrack]);
    if (!band) continue;
    const id = newId();
    const list = adds.get(band.clip.id) ?? [];
    list.push({ id, pitch: n.pitch, velocity: n.velocity, start: contentAt(band.clip, s), duration: n.duration });
    adds.set(band.clip.id, list);
    ids.push(id);
    if (n.muted) muted.push(id);
  }
  const commands: Command[] = [...adds].map(([clip, notes]) => cmd("Note", { type: "Add", clip, notes }));
  if (muted.length) {
    commands.push(cmd("Note", { type: "Edit", edits: muted.map((id) => ({ id, pitch: null, velocity: null, start: null, duration: null, muted: true })) }));
  }
  return {
    command: commands.length ? cmd("Edit", { type: "Batch", label: label.text, key: label.key, commands }) : null,
    ids,
    section: { clip: MULTI_SECTION_KEY, start: at, end: at + cb.length, ...(cb.pitches ? { pitches: cb.pitches } : {}) },
  };
}

// ─── Commands: editor axis → clip content ─────────────────────────────────────────────

export interface MultiClipContext {
  /** The editor's clips by id. */
  clips: ReadonlyMap<ClipId, Clip>;
  /** Clip of a note the project does not have yet (created earlier in this gesture). */
  pending: Map<NoteId, ClipId>;
}

function clipOfNote(project: Project | null, ctx: MultiClipContext, id: NoteId): Clip | undefined {
  const clip = project?.notes[id]?.clip ?? ctx.pending.get(id);
  return clip ? ctx.clips.get(clip) : undefined;
}

/** Group note ids by their clip (in the editor's clip order; unknown notes are dropped). */
function byClip(project: Project | null, ctx: MultiClipContext, ids: ReadonlyArray<NoteId> | null): Map<ClipId, NoteId[] | null> {
  const groups = new Map<ClipId, NoteId[] | null>();
  if (ids === null) {
    for (const id of ctx.clips.keys()) groups.set(id, null);
    return groups;
  }
  for (const id of ids) {
    const clip = clipOfNote(project, ctx, id);
    if (!clip) continue;
    const g = groups.get(clip.id) ?? [];
    g.push(id);
    groups.set(clip.id, g);
  }
  return groups;
}

const batch = (label: string, key: TextKey, commands: Command[]): Command =>
  commands.length === 1 ? commands[0]! : cmd("Edit", { type: "Batch", label, key, commands });

/**
 * Map a command sent in the multi-clip editor's (song) axis to clip content:
 * - `Note.Edit`: every `start` loses its note's clip shift;
 * - `Note.Duplicate`: the copies are remembered with their clip (later edits in the gesture);
 * - `Note.Quantize` / `Groove.Humanize` naming one of the clips: one per clip of the notes
 *   (every clip of the editor when `notes` is null), in one batch;
 * - `Edit.Batch`: mapped recursively. Anything else is unchanged (`Note.Add` already names
 *   its clip and content positions; expression commands are per clip).
 */
export function toClipContent(command: Command, ctx: MultiClipContext, project: Project | null): Command {
  if (command.domain === "Edit" && command.command.type === "Batch") {
    return cmd("Edit", { ...command.command, commands: command.command.commands.map((c) => toClipContent(c, ctx, project)) });
  }
  if (command.domain === "Note") {
    const c = command.command;
    if (c.type === "Edit") {
      return cmd("Note", {
        type: "Edit",
        edits: c.edits.map((e) => {
          if (e.start === null) return e;
          const clip = clipOfNote(project, ctx, e.id);
          return clip ? { ...e, start: Math.max(0, e.start - clipShift(clip)) } : e;
        }),
      });
    }
    if (c.type === "Duplicate") {
      for (const { from, new_id } of c.copies) {
        const clip = clipOfNote(project, ctx, from);
        if (clip) ctx.pending.set(new_id, clip.id);
      }
      return command;
    }
    if (c.type === "Quantize" && ctx.clips.has(c.clip)) {
      const parts = [...byClip(project, ctx, c.notes)].map(([clip, notes]) => cmd("Note", { ...c, clip, notes }));
      return parts.length ? batch("quantize notes", { id: "undo.note.quantize", args: {} }, parts) : command; // i18n-ignore
    }
  }
  if (command.domain === "Groove" && command.command.type === "Humanize" && ctx.clips.has(command.command.clip)) {
    const c = command.command;
    const parts = [...byClip(project, ctx, c.notes)].map(([clip, notes]) => cmd("Groove", { ...c, clip, notes }));
    return parts.length ? batch("humanize notes", { id: "undo.groove.humanize", args: {} }, parts) : command; // i18n-ignore
  }
  return command;
}

/**
 * `transport` for the multi-clip editor's components: commands are mapped with
 * `toClipContent` before being sent; everything else is the wrapped transport.
 */
export function multiClipTransport(transport: EngineTransport, context: () => Omit<MultiClipContext, "pending">): EngineTransport {
  const pending = new Map<NoteId, ClipId>();
  const send = (command: Command, opts?: SendOptions) => {
    if (pending.size > 4096) pending.clear();
    const mapped = toClipContent(command, { ...context(), pending }, useProjectStore.getState().project);
    return transport.send(mapped, opts);
  };
  return new Proxy(transport, {
    get(target, key) {
      if (key === "send") return send;
      const value: unknown = Reflect.get(target, key, target);
      return typeof value === "function" ? (value as (...args: unknown[]) => unknown).bind(target) : value;
    },
  });
}
