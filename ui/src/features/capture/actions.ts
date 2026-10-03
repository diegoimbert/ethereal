// Capture MIDI (`capture-midi`, CONTRACTS.md §13.4): turn what was just played into a clip.
import type { ClipId, Project, Track, TrackId } from "@/generated";
import { addTrack } from "@/features/arrangement/actions";
import { useArrangementUi } from "@/features/arrangement/state";
import { tracksOrdered, useProjectStore, useSelectionStore } from "@/state";
import { itemSelection } from "@/timeline/selection";
import { cmd, CommandFailedError, newId, type EngineTransport } from "@/transport";

/**
 * Which track a capture goes to: the selected track if it is a MIDI track, else the first
 * armed MIDI track, else the first MIDI track. `undefined`: there is no MIDI track.
 */
export function captureTargetTrack(project: Project, armed: ReadonlyArray<TrackId>): Track | undefined {
  const focus = useArrangementUi.getState().trackFocus ?? useSelectionStore.getState().selectedTrack;
  const selected = focus ? project.tracks[focus] : undefined;
  if (selected?.kind === "Midi") return selected;
  const midi = tracksOrdered(project).filter((t) => t.kind === "Midi");
  return midi.find((t) => armed.includes(t.id)) ?? midi[0];
}

/**
 * Like Ableton, the tempo is only inferred (and the loop set) when the arrangement is empty:
 * capturing into an existing song never changes its tempo.
 */
export function shouldAdoptTempo(project: Project): boolean {
  return Object.keys(project.clips).length === 0;
}

/**
 * Capture into the target track (a new MIDI track when there is none), then select the new
 * clip. Resolves with the clip id, or `null` when there is no project. Rejects like the
 * command (`InvalidState`: nothing was played).
 */
export async function captureMidi(transport: EngineTransport): Promise<ClipId | null> {
  const { project, armedTracks } = useProjectStore.getState();
  if (!project) return null;
  const adopt_tempo = shouldAdoptTempo(project);
  let track = captureTargetTrack(project, armedTracks)?.id;
  if (!track) {
    // Only add a track when there is something to put on it.
    const status = await transport.send(cmd("Capture", { type: "Status" }));
    if (status.type !== "CaptureStatus" || !status.status.available) {
      throw new CommandFailedError({ code: "InvalidState", message: "nothing was played to capture" });
    }
    track = await addTrack(transport, "Midi");
  }
  const clip = newId();
  await transport.send(cmd("Capture", { type: "Capture", track, clip, seed_notes: newId(), adopt_tempo }));
  if (useProjectStore.getState().project?.clips[clip]) itemSelection.getState().select("clip", [clip]);
  return clip;
}
