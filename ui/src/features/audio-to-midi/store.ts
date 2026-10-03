/**
 * Audio to MIDI from the UI (v0.3, `audio-to-midi`; CONTRACTS.md §13.5): the dialog state,
 * the remembered settings and the conversion in flight. The engine runs one conversion at a
 * time, off the audio thread, reporting `Event::AudioToMidi` progress; the document changes
 * once, at `Done` (one undo step), after which the new clip is selected.
 */

import { create } from "zustand";
import type {
  AudioToMidiMode,
  AudioToMidiOptions,
  Clip,
  ClipId,
  Event,
  TrackId,
} from "@/generated";
import { useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, isCommandFailed, newId, type EngineTransport } from "@/transport";

/** The engine's neutral defaults (`AudioToMidiOptions::default`). */
export const DEFAULT_OPTIONS: AudioToMidiOptions = {
  sensitivity: 0.5,
  min_duration: 0.05,
  min_pitch: 21,
  max_pitch: 108,
  kick_key: 36,
  snare_key: 38,
  hihat_key: 42,
};

export interface Settings {
  mode: AudioToMidiMode;
  options: AudioToMidiOptions;
  /** Add a default instrument (Poly Synth, or an empty Drum Rack for drums). */
  instrument: boolean;
}

export interface Conversion {
  job: string;
  clip: ClipId;
  mode: AudioToMidiMode;
  /** 0..=1. */
  progress: number;
  track: TrackId;
  newClip: ClipId;
}

interface AudioToMidiState {
  /** The clip whose Convert dialog is open. */
  dialog: ClipId | null;
  settings: Settings;
  job: Conversion | null;
  /** The last failure, shown on its clip (and in its dialog) until dismissed. */
  error: { clip: ClipId; message: string } | null;
}

const INITIAL: AudioToMidiState = {
  dialog: null,
  settings: { mode: "Melody", options: DEFAULT_OPTIONS, instrument: true },
  job: null,
  error: null,
};

export const useAudioToMidi = create<AudioToMidiState>()(() => ({ ...INITIAL }));

/** Forget everything (tests, a new connection). */
export function resetAudioToMidi(): void {
  useAudioToMidi.setState({ ...INITIAL });
}

export function openConvertDialog(clip: ClipId): void {
  useAudioToMidi.setState({ dialog: clip });
}

export function closeConvertDialog(): void {
  useAudioToMidi.setState({ dialog: null });
}

export function updateSettings(patch: Partial<Settings>): void {
  useAudioToMidi.setState((s) => ({ settings: { ...s.settings, ...patch } }));
}

export function updateOptions(patch: Partial<AudioToMidiOptions>): void {
  useAudioToMidi.setState((s) => ({ settings: { ...s.settings, options: { ...s.settings.options, ...patch } } }));
}

export function dismissError(): void {
  useAudioToMidi.setState({ error: null });
}

/** An audio clip on the main lane whose sample is known. */
export function isConvertible(clip: Clip): boolean {
  if (clip.content.type !== "Audio" || clip.lane) return false;
  return !!useProjectStore.getState().project?.media[clip.content.media];
}

const errorText = (err: unknown): string =>
  isCommandFailed(err) ? err.error.message || err.error.code : err instanceof Error ? err.message : String(err);

function ended(job: string): Conversion | null {
  const cur = useAudioToMidi.getState().job;
  return cur?.job === job ? cur : null;
}

/** Apply an engine event (exported for tests). */
export function applyAudioToMidiEvent(e: Event): void {
  if (e.type !== "AudioToMidi") return;
  const ev = e.event;
  const job = ended(ev.job);
  if (!job) return;
  switch (ev.type) {
    case "Progress":
      useAudioToMidi.setState({ job: { ...job, progress: ev.progress } });
      return;
    case "Done":
      useAudioToMidi.setState((s) => ({ job: null, dialog: s.dialog === job.clip ? null : s.dialog }));
      itemSelection.getState().select("clip", [ev.clip]);
      return;
    case "Failed":
      useAudioToMidi.setState({ job: null, error: { clip: job.clip, message: ev.message } });
      return;
    case "Cancelled":
      useAudioToMidi.setState({ job: null });
      return;
  }
}

const listening = new WeakSet<EngineTransport>();

function listen(transport: EngineTransport): void {
  if (listening.has(transport)) return;
  listening.add(transport);
  transport.onEvent(applyAudioToMidiEvent);
}

/** Start converting `clip` with the current settings (the engine replies at once). */
export async function startConversion(transport: EngineTransport, clip: ClipId): Promise<void> {
  if (useAudioToMidi.getState().job) return;
  listen(transport);
  const { settings } = useAudioToMidi.getState();
  const job: Conversion = {
    job: newId(),
    clip,
    mode: settings.mode,
    progress: 0,
    track: newId(),
    newClip: newId(),
  };
  useAudioToMidi.setState({ job, error: null });
  try {
    await transport.send(
      cmd("AudioToMidi", {
        type: "Start",
        job: job.job,
        clip,
        mode: settings.mode,
        options: settings.options,
        track: job.track,
        new_clip: job.newClip,
        seed_notes: newId(),
        instrument: settings.instrument ? newId() : null,
      }),
    );
  } catch (err) {
    if (ended(job.job)) useAudioToMidi.setState({ job: null, error: { clip, message: errorText(err) } });
  }
}

/** Stop the running conversion (the document is left as it was). */
export async function cancelConversion(transport: EngineTransport): Promise<void> {
  const job = useAudioToMidi.getState().job;
  if (!job) return;
  await transport.send(cmd("AudioToMidi", { type: "Cancel", job: job.job })).catch(() => {});
}
