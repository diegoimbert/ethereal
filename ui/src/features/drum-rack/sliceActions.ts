/**
 * Slicing a sample onto drum pads (`Slice::SampleToPads`), from the drum rack window, the
 * browser's sample menu and the audio clip menu. Each action is one undo step: the import
 * (if needed), the new track and rack (if any) and the pads share one gesture.
 *
 * Transient detection reads the media's peaks, which the engine computes after an import:
 * while they're missing the command fails with `error.drum_rack.sample_loading`, and the
 * action waits for `Media::PeaksReady` and sends it again.
 */

import type { AutoSlice, Command, DeviceId, MediaId, Project, Track, TrackId } from "@/generated";
import { resolveDroppedMedia, waitForMediaLength, type BrowserDragPayload } from "@/features/browser/dragPayload";
import { cmd, isCommandFailed, newId, nextGestureId, type EngineTransport } from "@/transport";
import { DEFAULT_BANK_START, slicePadIds } from "./padUtils";

/** What "slice to pads" uses: transients at medium sensitivity (the slice editor's default). */
export const DEFAULT_AUTO_SLICE: AutoSlice = { type: "Transients", sensitivity: 0.5 };
/** How long an action waits for a new sample's peaks. */
export const PEAKS_TIMEOUT_MS = 60_000;
const SAMPLE_LOADING = "error.drum_rack.sample_loading";

/** Did `err` fail because the media's peaks aren't computed yet? */
export function isSampleLoading(err: unknown): boolean {
  return isCommandFailed(err, "InvalidState") && err.error.key?.id === SAMPLE_LOADING;
}

/**
 * Send `command`, which reads `media`'s peaks (transient detection): if they are still being
 * computed, wait for them (at most `timeoutMs`) and send it again.
 */
export async function sendWithPeaks(
  transport: EngineTransport,
  command: Command,
  media: MediaId,
  opts: { gesture?: number; timeoutMs?: number } = {},
): Promise<void> {
  // Listening before the first send: the peaks may land between the failure and the wait.
  let ready = false;
  let wake: (() => void) | null = null;
  const off = transport.onEvent((e) => {
    if (e.type === "Media" && e.event.type === "PeaksReady" && e.event.media === media) {
      ready = true;
      wake?.();
    }
  });
  const send = () => transport.send(command, opts.gesture === undefined ? undefined : { gesture: opts.gesture });
  try {
    try {
      await send();
      return;
    } catch (err) {
      if (!isSampleLoading(err)) throw err;
    }
    if (!ready) {
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error("the sample's waveform is still loading: try again in a moment")), opts.timeoutMs ?? PEAKS_TIMEOUT_MS);
        wake = () => {
          clearTimeout(timer);
          resolve();
        };
      });
    }
    await send();
  } finally {
    off();
  }
}

/** The pads of `rack`, sorted by note. */
function padNotes(project: Project, rack: DeviceId): number[] {
  return Object.values(project.drum_pads)
    .filter((p) => p.rack === rack)
    .map((p) => p.note)
    .sort((a, b) => a - b);
}

/** Where slices go on `rack`: above its highest pad (C1 on an empty rack); `null` when no key is left. */
export function firstFreeNote(project: Project, rack: DeviceId): number | null {
  const notes = padNotes(project, rack);
  const next = notes.length === 0 ? DEFAULT_BANK_START : notes[notes.length - 1]! + 1;
  return next <= 127 ? next : null;
}

/** `Slice::SampleToPads` for every slice from `base` up to key 127. */
export function samplePadsCommand(rack: DeviceId, media: MediaId, base: number, mode: AutoSlice = DEFAULT_AUTO_SLICE): Command {
  return cmd("Slice", { type: "SampleToPads", rack, media, mode, base_note: base, pads: slicePadIds(128 - base) });
}

/** The project media of a browser sample (imported if needed, its length known). */
async function mediaOf(transport: EngineTransport, payload: BrowserDragPayload, gesture: number): Promise<MediaId> {
  const media = await resolveDroppedMedia(transport, payload, { gesture });
  if (media.frames === 0) await waitForMediaLength(transport, media.id);
  return media.id;
}

async function inGesture<T>(transport: EngineTransport, run: (gesture: number) => Promise<T>): Promise<T> {
  const gesture = nextGestureId();
  try {
    return await run(gesture);
  } finally {
    transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
  }
}

/** Slice a browser sample onto `rack`'s pads, from `base` (default: above its pads). One undo step. */
export function slicePayloadToPads(transport: EngineTransport, project: Project, rack: DeviceId, payload: BrowserDragPayload, base?: number): Promise<void> {
  const note = base ?? firstFreeNote(project, rack);
  if (note === null) return Promise.reject(new Error("this drum rack has no free keys left"));
  return inGesture(transport, async (gesture) => {
    const media = await mediaOf(transport, payload, gesture);
    await sendWithPeaks(transport, samplePadsCommand(rack, media, note), media, { gesture });
  });
}

/** The next sibling of `track` that a regular track can be put before (`null`: the end). */
function nextSibling(project: Project, track: Track): TrackId | null {
  const siblings = Object.values(project.tracks)
    .filter((t) => t.parent === track.parent)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1));
  const next = siblings[siblings.indexOf(track) + 1];
  return next && next.kind !== "Return" && next.kind !== "Master" ? next.id : null;
}

/**
 * A new MIDI track (named `name`, after `after` when given, else at the end) whose drum rack
 * holds `media`'s slices from C1: one undo step.
 */
export async function sliceMediaToNewTrack(
  transport: EngineTransport,
  project: Project,
  media: MediaId,
  opts: { name?: string | null; after?: TrackId; gesture?: number } = {},
): Promise<TrackId> {
  const after = opts.after ? project.tracks[opts.after] : undefined;
  const track = newId();
  const rack = newId();
  const command = cmd("Edit", {
    type: "Batch",
    label: "slice to drum rack", // i18n-ignore: the English undo label, keyed below
    key: { id: "undo.slice.to_drum_rack", args: {} },
    commands: [
      cmd("Track", {
        type: "Create",
        id: track,
        kind: "Midi",
        name: opts.name ?? null,
        color: null,
        parent: after?.parent ?? null,
        before: after ? nextSibling(project, after) : null,
      }),
      cmd("Device", { type: "Insert", id: rack, track, device: { type: "Builtin", device: { type: "DrumRack" } }, before: null }),
      samplePadsCommand(rack, media, DEFAULT_BANK_START),
    ],
  });
  await sendWithPeaks(transport, command, media, { gesture: opts.gesture });
  return track;
}

/** Slice a browser sample to a new drum rack track (named after the sample). One undo step. */
export function slicePayloadToNewTrack(transport: EngineTransport, project: Project, payload: BrowserDragPayload): Promise<TrackId> {
  return inGesture(transport, async (gesture) => {
    const media = await mediaOf(transport, payload, gesture);
    return sliceMediaToNewTrack(transport, project, media, { name: payload.name.replace(/\.[^.]+$/, ""), gesture });
  });
}
