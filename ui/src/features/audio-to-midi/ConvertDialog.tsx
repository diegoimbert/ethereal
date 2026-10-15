import type { AudioToMidiMode, Clip } from "@/generated";
import { pitchName } from "@/features/piano-roll/geometry";
import { Button, Dialog, NumberField, Toggle } from "@/kit";
import type { EngineTransport } from "@/transport";
import { Trans, useT, type MessageKey } from "@/i18n";
import { cancelConversion, closeConvertDialog, DEFAULT_OPTIONS, dismissError, startConversion, updateOptions, updateSettings, useAudioToMidi } from "./store";

const MODES: ReadonlyArray<{ mode: AudioToMidiMode; label: MessageKey; help: MessageKey }> = [
  { mode: "Melody", label: "audio_to_midi.mode.melody", help: "audio_to_midi.mode.melody_help" },
  { mode: "Harmony", label: "audio_to_midi.mode.harmony", help: "audio_to_midi.mode.harmony_help" },
  { mode: "Drums", label: "audio_to_midi.mode.drums", help: "audio_to_midi.mode.drums_help" },
];

/** The drum keys' fields. */
const DRUM_KEYS = [
  ["kick_key", "audio_to_midi.drum.kick"],
  ["snare_key", "audio_to_midi.drum.snare"],
  ["hihat_key", "audio_to_midi.drum.hihat"],
] as const;

/**
 * "Convert to MIDI" for one audio clip: the kind of material, the detection settings and
 * whether to add an instrument. While converting it shows the progress (Stop cancels; Hide
 * keeps it running, with progress on the clip); it closes when the new track is in.
 */
export function ConvertDialog({ clip, transport }: { clip: Clip; transport: EngineTransport }) {
  const t = useT();
  const open = useAudioToMidi((s) => s.dialog === clip.id);
  const settings = useAudioToMidi((s) => s.settings);
  const job = useAudioToMidi((s) => s.job);
  const error = useAudioToMidi((s) => (s.error?.clip === clip.id ? s.error.message : null));
  const mine = job?.clip === clip.id ? job : null;
  const busyElsewhere = !!job && !mine;
  const o = settings.options;
  const drums = settings.mode === "Drums";
  const name = clip.name || t("audio_to_midi.dialog.this_clip");

  const footer = mine ? (
    <>
      <Button size="sm" onClick={closeConvertDialog}>
        {t("audio_to_midi.dialog.hide")}
      </Button>
      <Button size="sm" tone="danger" onClick={() => void cancelConversion(transport)}>
        {t("audio_to_midi.dialog.stop")}
      </Button>
    </>
  ) : (
    <>
      <Button size="sm" tone="ghost" onClick={closeConvertDialog}>
        {t("audio_to_midi.dialog.cancel")}
      </Button>
      <Button
        size="sm"
        tone="accent"
        disabled={busyElsewhere}
        title={busyElsewhere ? t("audio_to_midi.dialog.busy") : undefined}
        onClick={() => void startConversion(transport, clip.id)}
      >
        {t("audio_to_midi.dialog.convert")}
      </Button>
    </>
  );

  return (
    <Dialog
      open={open}
      onClose={closeConvertDialog}
      title={<Trans k="audio_to_midi.dialog.title" values={{ name: <span className="eth-user-text">{name}</span> }} />}
      footer={footer}
      className="eth-a2m"
    >
      <div className="eth-a2m__form" data-testid="audio-to-midi-dialog">
        <div className="eth-a2m__modes" role="radiogroup" aria-label={t("audio_to_midi.dialog.material")}>
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
              {t(m.label)}
            </Button>
          ))}
        </div>
        <p className="eth-a2m__help">{t(MODES.find((m) => m.mode === settings.mode)!.help)}</p>

        <fieldset className="eth-a2m__fields" disabled={!!mine}>
          <label className="eth-a2m__row">
            <span>{t("audio_to_midi.dialog.sensitivity")}</span>
            <NumberField
              size="sm"
              aria-label={t("audio_to_midi.dialog.sensitivity")}
              value={Math.round(o.sensitivity * 100)}
              min={0}
              max={100}
              unit="%"
              onChange={(v) => updateOptions({ sensitivity: v / 100 })}
            />
          </label>
          {drums ? (
            DRUM_KEYS.map(([key, label]) => (
              <label key={key} className="eth-a2m__row">
                <span>{t(label)}</span>
                <NumberField
                  size="sm"
                  aria-label={t(label)}
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
                <span>{t("audio_to_midi.dialog.shortest")}</span>
                <NumberField
                  size="sm"
                  aria-label={t("audio_to_midi.dialog.shortest")}
                  value={Math.round(o.min_duration * 1000)}
                  min={0}
                  max={2000}
                  step={10}
                  unit="ms"
                  onChange={(v) => updateOptions({ min_duration: v / 1000 })}
                />
              </label>
              <div className="eth-a2m__row">
                <span>{t("audio_to_midi.dialog.range")}</span>
                <span className="eth-a2m__range">
                  <NumberField
                    size="sm"
                    aria-label={t("audio_to_midi.dialog.lowest")}
                    value={o.min_pitch}
                    min={0}
                    max={o.max_pitch}
                    unit={pitchName(o.min_pitch)}
                    onChange={(v) => updateOptions({ min_pitch: v })}
                  />
                  <span aria-hidden>{t("audio_to_midi.dialog.to")}</span>
                  <NumberField
                    size="sm"
                    aria-label={t("audio_to_midi.dialog.highest")}
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
            <span>{t("audio_to_midi.dialog.instrument")}</span>
            <Toggle
              size="sm"
              checked={settings.instrument}
              label={drums ? t("audio_to_midi.dialog.add_drum_rack") : t("audio_to_midi.dialog.add_synth")}
              disabled={!!mine}
              onChange={(instrument) => updateSettings({ instrument })}
            />
          </div>
          <div className="eth-a2m__reset">
            <Button size="sm" tone="ghost" onClick={() => updateOptions(DEFAULT_OPTIONS)}>
              {t("audio_to_midi.dialog.reset")}
            </Button>
          </div>
        </fieldset>

        {mine && (
          <div className="eth-a2m__status" role="status">
            <span>{t("audio_to_midi.dialog.analysing", { pct: Math.round(mine.progress * 100) })}</span>
            <span
              className="eth-a2m__bar"
              role="progressbar"
              aria-label={t("audio_to_midi.dialog.converting", { name })}
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
              {t("common.dismiss")}
            </Button>
          </p>
        )}
      </div>
    </Dialog>
  );
}
