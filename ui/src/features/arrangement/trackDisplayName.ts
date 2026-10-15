import type { BuiltinDeviceType, Track } from "@/generated";
import { useProjectStore } from "@/state";

/** A MIDI track's name as the engine gives it ("3 midi"): an unnamed track. */
const DEFAULT_MIDI_NAME = /^\d+ midi$/i;
/** Built-in instruments (an unnamed track shows its instrument's name). */
const INSTRUMENTS = new Set<BuiltinDeviceType>([
  "Synth",
  "PolySynth",
  "FmSynth",
  "Sampler",
  "MultiSampler",
  "DrumRack",
  "InstrumentRack",
  "ExternalInstrument",
]);

/**
 * The name a track header shows: its own, or for an unnamed MIDI track (still its default
 * "N midi") its instrument's name (the first built-in instrument on its chain, else its first
 * plugin). Renaming the track gives it its own name back.
 */
export function useTrackDisplayName(track: Pick<Track, "id" | "name" | "kind">): string {
  const instrument = useProjectStore((s) => {
    if (!s.project || track.kind !== "Midi" || !DEFAULT_MIDI_NAME.test(track.name)) return null;
    const own = Object.values(s.project.devices).filter((d) => d.track === track.id && d.chain === null && d.pad === null);
    const found = own.find((d) => d.kind.type === "Builtin" && INSTRUMENTS.has(d.kind.device.type)) ?? own.find((d) => d.kind.type === "Plugin");
    return found?.name ?? null;
  });
  return instrument ?? track.name;
}
