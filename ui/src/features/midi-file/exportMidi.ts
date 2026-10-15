/**
 * MIDI file export (#717): "export MIDI…" on a clip or a selection of clips (the file starts
 * at the first clip) and on tracks (all their MIDI clips, from the project start, so they
 * keep their place). The engine writes a type 1 Standard MIDI File (`Media::WriteMidiFile`):
 * the desktop asks where in its OS save dialog and the engine writes it there; elsewhere the
 * bytes come back and are saved as a download.
 */
import type { Clip, ClipId, Project, TrackId } from "@/generated";
import { saveFile } from "@/features/export/download";
import { notify } from "@/features/notifications";
import { cmd, commandErrorText, isCommandFailed, type EngineTransport } from "@/transport";
import { t } from "@/i18n";

/** The desktop transport's save dialog for MIDI files (`TauriTransport`). */
export interface MidiFileHost {
  /** An absolute path ending in `.mid`, or `null` when dismissed. */
  pickMidiSavePath(defaultName: string): Promise<string | null>;
}

export function isMidiFileHost(t: EngineTransport | null | undefined): t is EngineTransport & MidiFileHost {
  return typeof (t as Partial<MidiFileHost> | null | undefined)?.pickMidiSavePath === "function";
}

function base64ToBytes(data: string): Uint8Array {
  const bin = atob(data);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** The MIDI clips of `tracks` (main lanes only: take lanes don't play), by start. */
export function midiClipsOfTracks(project: Project, tracks: ReadonlyArray<TrackId>): Clip[] {
  const set = new Set(tracks);
  return Object.values(project.clips)
    .filter((c) => set.has(c.track) && c.content.type === "Midi" && !c.lane)
    .sort((a, b) => a.start - b.start);
}

/**
 * Export `clips` as a MIDI file named `name` (see the module docs). Resolves `false` when the
 * save dialog was dismissed; a failure is shown as an error notice.
 */
export async function exportMidiClips(
  transport: EngineTransport,
  clips: ReadonlyArray<ClipId>,
  opts: { name: string; fromProjectStart: boolean },
): Promise<boolean> {
  try {
    let path: string | null = null;
    if (isMidiFileHost(transport)) {
      path = await transport.pickMidiSavePath(`${opts.name || "midi"}.mid`);
      if (!path) return false;
    }
    const reply = await transport.send(
      cmd("Media", { type: "WriteMidiFile", request: { clips: [...clips], from_project_start: opts.fromProjectStart, name: opts.name, path } }),
    );
    if (reply.type !== "MidiFileWritten") throw new Error(`unexpected reply ${reply.type}`);
    const { result } = reply;
    if (result.data) saveFile(result.name, "audio/midi", base64ToBytes(result.data));
    notify("Info", result.path ? t("midi_file.exported_to", { name: result.name, path: result.path }) : t("midi_file.exported", { name: result.name }));
    return true;
  } catch (e) {
    notify("Error", t("midi_file.export_failed", { error: isCommandFailed(e) ? commandErrorText(e.error) : e instanceof Error ? e.message : String(e) }));
    return false;
  }
}
