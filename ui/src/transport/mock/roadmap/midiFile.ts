/**
 * Mock `Media::ReadMidiFile` / `WriteMidiFile` (#717). The mock has no SMF codec (the
 * engine's is `ether_media::smf`): any MIDI file reads as the canned two-track groove below
 * (named after the file), and a written file is a stub header. Validation follows the
 * controller (`ether-controller/src/midi_file.rs`).
 */
import type { MediaSource, MidiExportRequest, MidiExportResult, MidiFileContents, MidiFileNote, Project } from "@/generated";
import { bytesToBase64 } from "@/transport/ws/binaryFrame";
import { fail } from "../documentReducer";

const isMidi = (name: string) => /\.midi?$/i.test(name);

const note = (pitch: number, start: number, duration: number, velocity = 0.8): MidiFileNote => ({ pitch, velocity, release_velocity: 0.5, start, duration });

/** What every MIDI file reads as in the mock: one bar of drums (channel 10) and bass. */
export function cannedMidiFile(fileName: string): MidiFileContents {
  return {
    name: fileName.replace(/\.[^.]*$/, "") || fileName,
    tracks: [
      { name: "drums", channel: 9, notes: [0, 1, 2, 3].map((b) => note(36, b, 0.25, 1)) },
      { name: "bass", channel: 0, notes: [note(36, 0, 1.5), note(43, 2, 1.5)] },
    ],
    tempos: [{ time: 0, bpm: 100 }],
    time_signatures: [{ time: 0, signature: { numerator: 4, denominator: 4 } }],
    length: 3.5,
  };
}

/** The file name a source names (validated like the controller); `take` consumes an upload. */
export function readMockMidiFile(source: MediaSource, uploadName: (upload: string) => string): MidiFileContents {
  let name: string;
  switch (source.type) {
    case "Project":
      return fail("InvalidArgument", "project media are audio, not MIDI files");
    case "Path":
      return fail("Unsupported", "importing OS files by path needs the desktop engine");
    case "Upload":
      name = uploadName(source.upload);
      break;
    case "Location":
      name = source.path.split("/").pop() ?? source.path;
      break;
  }
  if (!isMidi(name)) fail("InvalidArgument", `not a MIDI file: ${name}`);
  return cannedMidiFile(name);
}

/** `ether_controller::midi_file::midi_file_name`. */
export function midiFileName(name: string): string {
  const clean = name
    // eslint-disable-next-line no-control-regex
    .replace(/[\u0000-\u001f\u007f/\\:*?"<>|]/g, "_")
    .trim()
    .replace(/^\.+/, "")
    .trim();
  return `${clean || "midi"}.mid`;
}

export function writeMockMidiFile(project: Project, r: MidiExportRequest): MidiExportResult {
  if (r.clips.length === 0) fail("InvalidArgument", "no clips to export");
  for (const id of r.clips) {
    const c = project.clips[id] ?? fail("NotFound", `clip ${id}`);
    if (c.content.type !== "Midi") fail("InvalidArgument", `clip ${id} is an audio clip`);
  }
  if (r.path !== null) fail("Unsupported", "MIDI files are returned as bytes on this host");
  const header = new TextEncoder().encode("MThd");
  return { name: midiFileName(r.name), path: null, data: bytesToBase64(header) };
}
