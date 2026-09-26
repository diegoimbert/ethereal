/**
 * The mock engine's fake media library: one engine-visible browse root ("Library") with a
 * few folders of audio files. Only metadata exists (frames / sample rate / channels);
 * peaks for imported files are synthesized (`peaks.ts`).
 *
 * All paths are relative to their browse location (never file-system paths): the UI may
 * run on another machine than the engine.
 */

import type { BrowseRoot, DirectoryEntry, FileKind } from "@/generated";

export const LIBRARY_ID = "library";

export const MOCK_LOCATIONS: ReadonlyArray<BrowseRoot> = [
  { location: { type: "Library", id: LIBRARY_ID }, name: "Library" },
  { location: { type: "ProjectMedia" }, name: "Project media" },
];

export interface LibraryFile {
  /** Relative path inside the library, `/`-separated. */
  path: string;
  kind: FileKind;
  sample_rate: number;
  channels: number;
  frames: number;
}

const wav = (path: string, seconds: number, channels = 2, sample_rate = 44100): LibraryFile => ({
  path,
  kind: "Audio",
  sample_rate,
  channels,
  frames: Math.round(seconds * sample_rate),
});

export const LIBRARY_FILES: ReadonlyArray<LibraryFile> = [
  wav("Drums/Kick.wav", 0.6, 1),
  wav("Drums/Snare.wav", 0.5, 1),
  wav("Drums/Hat Closed.wav", 0.2, 1),
  wav("Drums/Loops/Break 120.wav", 8),
  wav("Drums/Loops/Shuffle 96.wav", 10),
  wav("Synths/Pad C.wav", 12, 2, 48000),
  wav("Synths/Bass A1.wav", 2, 1),
  wav("Vocals/Chop 1.wav", 1.5),
  wav("Vocals/Phrase 2.wav", 4),
  { path: "MIDI/Groove 1.mid", kind: "Midi", sample_rate: 0, channels: 0, frames: 0 },
  { path: "Readme.txt", kind: "Other", sample_rate: 0, channels: 0, frames: 0 },
];

/** Approximate size in bytes of a 16-bit PCM WAV file. */
export function wavSize(frames: number, channels: number): number {
  return frames * channels * 2 + 44;
}

function fileSize(f: LibraryFile): number {
  if (f.kind === "Audio") return wavSize(f.frames, f.channels);
  return f.kind === "Midi" ? 512 : 1024;
}

export function findLibraryFile(path: string): LibraryFile | undefined {
  return LIBRARY_FILES.find((f) => f.path === normalize(path));
}

/** `"/Drums//Loops/"` → `"Drums/Loops"`. */
export function normalize(path: string): string {
  return path.split("/").filter(Boolean).join("/");
}

/** Entries of folder `path` (`""` = root): folders first, then files, by name. `null` if no such folder. */
export function listLibraryFolder(path: string): DirectoryEntry[] | null {
  const dir = normalize(path);
  const prefix = dir ? `${dir}/` : "";
  const folders = new Set<string>();
  const files: DirectoryEntry[] = [];
  let exists = dir === "";
  for (const f of LIBRARY_FILES) {
    if (!f.path.startsWith(prefix)) continue;
    exists = true;
    const rest = f.path.slice(prefix.length);
    const slash = rest.indexOf("/");
    if (slash >= 0) folders.add(rest.slice(0, slash));
    else files.push({ name: rest, path: f.path, kind: f.kind, size: fileSize(f) });
  }
  if (!exists) return null;
  const byName = (a: DirectoryEntry, b: DirectoryEntry) => a.name.localeCompare(b.name);
  return [
    ...[...folders].map((name): DirectoryEntry => ({ name, path: prefix + name, kind: "Directory", size: 0 })).sort(byName),
    ...files.sort(byName),
  ];
}
