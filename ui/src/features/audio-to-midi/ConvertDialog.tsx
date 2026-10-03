import type { AudioToMidiMode, Clip } from "@/generated";
import { pitchName } from "@/features/piano-roll/geometry";
import { Button, Dialog, NumberField, Toggle } from "@/kit";
import type { EngineTransport } from "@/transport";
import {
  cancelConversion,
  closeConvertDialog,
  DEFAULT_OPTIONS,
  dismissError,
  startConversion,
  updateOptions,
  updateSettings,
  useAudioToMidi,
} from "./store";

const MODES: ReadonlyArray<{ mode: AudioToMidiMode; label: string; help: string }> = [
  { mode: "Melody", label: "Melody", help: "One note at a time: vocals, bass lines, leads." },
  { mode: "Harmony", label: "Harmony", help: "Chords and several notes at once: piano, guitar, pads." },
  { mode: "Drums", label: "Drums", help: "Kick, snare and hi-hat hits, on drum keys." },
];

/**
 * "Convert to MIDI" for one audio clip: the kind of material, the detection settings and
 * whether to add an instrument. While converting it shows the progress (Stop cancels; Hide
 * keeps it running, with progress on the clip); it closes when the new track is in.
 */
export function ConvertDialog({ clip, transport }: { clip: Clip; transport: EngineTransport }) {
  const open = useAudioToMidi((s) => s.dialog === clip.id);
  const settings = useAudioToMidi((s) => s.settings);
  const job = useAudioToMidi((s) => s.job);
  const error = useAudioToMidi((s) => (s.error?.clip === clip.id ? s.error.message : null));
  const mine = job?.clip === clip.id ? job : null;
  const busyElsewhere = !!job && !mine;
  const o = settings.options;
  const drums = settings.mode === "Drums";
  const name = clip.name || "this clip";

  const footer = mine ? (
    <>
      <Button size="sm" onClick={closeConvertDialog}>
        Hide
      </Button>
      <Button size="sm" tone="danger" onClick={() => void cancelConversion(transport)}>
        Stop
      </Button>
    </>
  ) : (
    <>
      <Button size="sm" tone="ghost" onClick={closeConvertDialog}>
        Cancel
      </Button>
      <Button
        size="sm"
        tone="accent"
        disabled={busyElsewhere}
        title={busyElsewhere ? "Another conversion is running" : undefined}
        onClick={() => void startConversion(transport, clip.id)}
      >
        Convert
      </Button>
    </>
  );

  return (
    <Dialog open={open} onClose={closeConvertDialog} title={`Convert “${name}” to MIDI`} footer={footer} className="eth-a2m">
      <div className="eth-a2m__form" data-testid="audio-to-midi-dialog">
        <div className="eth-a2m__modes" role="radiogroup" aria-label="Material">
          {MODES.map((m) => (
            <Button
              key={m.mode}
              size="sm"
              role="radio"
              aria-checked={settings.mode === m.mode}
              active={settings.mode === m.mode}
              disabled={!!mine}
              onClick={() => updateSettings({ mode: m.mode })}
            >
              {m.label}
            </Button>
          ))}
        </div>
        <p className="eth-a2m__help">{MODES.find((m) => m.mode === settings.mode)!.help}</p>

        <fieldset className="eth-a2m__fields" disabled={!!mine}>
          <label className="eth-a2m__row">
            <span>Sensitivity</span>
            <NumberField
              size="sm"
              aria-label="Sensitivity"
              value={Math.round(o.sensitivity * 100)}
              min={0}
              max={100}
              unit="%"
              onChange={(v) => updateOptions({ sensitivity: v / 100 })}
            />
          </label>
          {drums ? (
            (
              [
                ["kick_key", "Kick"],
                ["snare_key", "Snare"],
                ["hihat_key", "Hi-hat"],
              ] as const
            ).map(([key, label]) => (
              <label key={key} className="eth-a2m__row">
                <span>{label} key</span>
                <NumberField
                  size="sm"
                  aria-label={`${label} key`}
                  value={o[key]}
                  min={0}
                  max={127}
                  unit={pitchName(o[key])}
                  onChange={(v) => updateOptions({ [key]: v })}
                />
              </label>
            ))
          ) : (
            <>
              <label className="eth-a2m__row">
                <span>Shortest note</span>
                <NumberField
                  size="sm"
                  aria-label="Shortest note"
                  value={Math.round(o.min_duration * 1000)}
                  min={0}
                  max={2000}
                  step={10}
                  unit="ms"
                  onChange={(v) => updateOptions({ min_duration: v / 1000 })}
                />
              </label>
              <div className="eth-a2m__row">
                <span>Pitch range</span>
                <span className="eth-a2m__range">
                  <NumberField
                    size="sm"
                    aria-label="Lowest note"
                    value={o.min_pitch}
                    min={0}
                    max={o.max_pitch}
                    unit={pitchName(o.min_pitch)}
                    onChange={(v) => updateOptions({ min_pitch: v })}
                  />
                  <span aria-hidden>to</span>
                  <NumberField
                    size="sm"
                    aria-label="Highest note"
                    value={o.max_pitch}
                    min={o.min_pitch}
                    max={127}
                    unit={pitchName(o.max_pitch)}
                    onChange={(v) => updateOptions({ max_pitch: v })}
                  />
                </span>
              </div>
            </>
          )}
          <div className="eth-a2m__row">
            <span>Instrument</span>
            <Toggle
              size="sm"
              checked={settings.instrument}
              label={drums ? "Add a Drum Rack" : "Add a Poly Synth"}
              disabled={!!mine}
              onChange={(instrument) => updateSettings({ instrument })}
            />
          </div>
          <div className="eth-a2m__reset">
            <Button size="sm" tone="ghost" onClick={() => updateOptions(DEFAULT_OPTIONS)}>
              Reset settings
            </Button>
          </div>
        </fieldset>

        {mine && (
          <div className="eth-a2m__status" role="status">
            <span>Analysing… {Math.round(mine.progress * 100)}%</span>
            <span
              className="eth-a2m__bar"
              role="progressbar"
              aria-label={`Converting ${name}`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(mine.progress * 100)}
            >
              <span className="eth-a2m__bar-fill" style={{ transform: `scaleX(${mine.progress})` }} />
            </span>
          </div>
        )}
        {error && !mine && (
          <p className="eth-a2m__error" role="alert">
            {error}{" "}
            <Button size="sm" tone="ghost" onClick={dismissError}>
              Dismiss
            </Button>
          </p>
        )}
      </div>
    </Dialog>
  );
}
