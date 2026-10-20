/**
 * The piano roll's time axis and edit targets: one clip (its content timeline, as always) or
 * several clips (multi-clip editing, the song timeline; see `multiClip.ts`). The
 * editor's components work in "editor beats" and ask the space where things go.
 */

import { useMemo } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Beats, Clip, ClipId, Note, TrackId } from "@/generated";
import { notesOfClip, tracksOrdered, useProjectStore } from "@/state";
import { useSelectedItems } from "@/timeline";
import { contentFloor, contentToSong, songToContent } from "./clipTime";
import { bandAt, bandLimits, clipBand, clipShift, contentAt, loopGhosts, MULTI_SECTION_KEY, songNotes, type ClipBand, type GhostNote } from "./multiClip";
import type { MoveLimits } from "./noteEdits";

export interface RollSpace {
  multi: boolean;
  /** Scope key of the section store (the clip id; `MULTI_SECTION_KEY` for several clips). */
  key: ClipId;
  /** The focused clip: the one toolbar, scale, expression lanes and collab presence follow. */
  focus: Clip;
  /** The edited clips (just `focus` for one clip). */
  clips: ReadonlyArray<Clip>;
  /** The clips' bands on the song axis (multi-clip only; empty otherwise). */
  bands: ReadonlyArray<ClipBand>;
  /** The clip edits may grow (a paste / stretch past its end); null with several clips. */
  grow: Clip | null;
  /** Editor beats − focused clip content beats (0 for one clip). */
  focusShift: Beats;
  /** Editor position of song beat `song` (null where the clip does not play). */
  fromSong(song: Beats): Beats | null;
  /** Song beat of editor position `beats`. */
  toSong(beats: Beats): Beats;
  /** The clip and content position a note drawn or pasted at `beats` lands in (null: none). */
  place(beats: Beats, tracks?: ReadonlyArray<TrackId>): { clip: Clip; content: Beats } | null;
  /** First editor beat of the axis: the clip's `contentFloor` (before content 0 when its left
   *  edge was extended past the content start), song beat 0 for several clips. */
  floor: Beats;
  /** Move limits of note starts (editor beats); undefined = from 0 on. */
  limits?: MoveLimits;
}

/** The space of a single clip: its content timeline (the piano roll as it always was). */
export function singleClipSpace(clip: Clip): RollSpace {
  const floor = contentFloor(clip);
  return {
    multi: false,
    key: clip.id,
    focus: clip,
    clips: [clip],
    bands: [],
    grow: clip,
    focusShift: 0,
    fromSong: (song) => songToContent(clip, song),
    toSong: (beats) => contentToSong(clip, beats),
    place: (beats) => ({ clip, content: Math.max(floor, beats) }),
    floor,
    limits: () => [floor, Infinity],
  };
}

/** The space of several clips (`focus` among them) on the song timeline. */
export function multiClipSpace(focus: Clip, clips: ReadonlyArray<Clip>): RollSpace {
  const bands = clips.map(clipBand);
  const byClip = new Map(bands.map((b) => [b.clip.id, b]));
  return {
    multi: true,
    key: MULTI_SECTION_KEY,
    focus,
    clips,
    bands,
    grow: null,
    floor: 0,
    focusShift: clipShift(focus),
    fromSong: (song) => song,
    toSong: (beats) => beats,
    place: (beats, tracks = []) => {
      const band = bandAt(bands, beats, [...tracks, focus.track, ...new Set(clips.map((c) => c.track))]);
      return band ? { clip: band.clip, content: contentAt(band.clip, beats) } : null;
    },
    limits: (note) => {
      const band = byClip.get(note.clip);
      return band ? bandLimits(band, note) : null;
    },
  };
}

/**
 * The MIDI clips edited together with `clip`: every selected MIDI clip when `clip` is one of
 * at least two (in track order, then time), else null (single-clip editing).
 */
export function useMultiClipSelection(clip: Clip | undefined): Clip[] | null {
  const selected = useSelectedItems("clip");
  const clips = useProjectStore(
    useShallow((s) => {
      const p = s.project;
      if (!p || !clip || !selected.has(clip.id) || selected.size < 2) return EMPTY;
      const midi = [...selected].map((id) => p.clips[id]).filter((c): c is Clip => !!c && c.content.type === "Midi");
      if (midi.length < 2) return EMPTY;
      const order = new Map(tracksOrdered(p).map((t, i) => [t.id, i]));
      return midi.sort((a, b) => (order.get(a.track) ?? 0) - (order.get(b.track) ?? 0) || a.start - b.start);
    }),
  );
  return clips.length ? clips : null;
}

const EMPTY: Clip[] = [];

/** The shown notes of the space's clips in editor beats, and the loop repetitions' ghosts. */
export function useSpaceNotes(space: RollSpace): { notes: Note[]; ghosts: GhostNote[] } {
  const ids = space.clips.map((c) => c.id);
  const raw = useProjectStore(useShallow((s) => (s.project ? ids.map((id) => notesOfClip(s.project!, id)) : EMPTY_LISTS)));
  return useMemo(() => {
    if (!space.multi) return { notes: raw[0] ?? EMPTY_NOTES, ghosts: [] };
    const lists = new Map(ids.map((id, i) => [id, raw[i] ?? []]));
    const of = (id: ClipId) => lists.get(id) ?? [];
    return { notes: songNotes(space.bands, of), ghosts: loopGhosts(space.bands, of) };
    // `ids` follows `space`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [raw, space]);
}

const EMPTY_LISTS: Note[][] = [];
const EMPTY_NOTES: Note[] = [];
