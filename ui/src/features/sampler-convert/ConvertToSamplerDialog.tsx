import { useState } from "react";
import { AudioLines, Info, Plus, X } from "lucide-react";
import type { Device } from "@/generated";
import { Button, Dialog, IconButton, NumberField, Popover, Select, Toggle, Tooltip } from "@/kit";
import { useTransport } from "@/transport";
import { useT } from "@/i18n";
import { SampleKeyboard } from "./SampleKeyboard";
import {
  KEY_PRESET_STEPS,
  MAX_CONVERT_SAMPLES,
  MAX_CONVERT_VELOCITIES,
  VELOCITY_PRESET_COUNTS,
  convertToSampler,
  presetKeys,
  presetVelocities,
  sampleCount,
  sortedSet,
  useSamplerConvert,
  velocityToAdd,
} from "./store";
import "./sampler-convert.css";

/** A preset's Select value ("custom" once the list is edited by hand). */
type PresetValue = string;
const CUSTOM = "custom";

const DEFAULT_KEY_STEP = 3;
const DEFAULT_VELOCITIES = 3;

/** The "convert to sampler" settings dialog, mounted once (`App`). */
export function ConvertToSamplerDialog() {
  const device = useSamplerConvert((s) => s.device);
  if (!device) return null;
  // Keyed by device: a new conversion starts from the defaults.
  return <ConvertDialog key={device.id} device={device} />;
}

function ConvertDialog({ device }: { device: Device }) {
  const t = useT();
  const transport = useTransport();
  const close = useSamplerConvert((s) => s.close);
  const [keys, setKeys] = useState(() => presetKeys(DEFAULT_KEY_STEP));
  const [keyPreset, setKeyPreset] = useState<PresetValue>(String(DEFAULT_KEY_STEP));
  const [velocities, setVelocities] = useState(() => presetVelocities(DEFAULT_VELOCITIES));
  const [velocityPreset, setVelocityPreset] = useState<PresetValue>(String(DEFAULT_VELOCITIES));
  const [preRollSeconds, setPreRollSeconds] = useState(0.2);
  const [warmUpNote, setWarmUpNote] = useState(false);
  const [noteSeconds, setNoteSeconds] = useState(4);
  const [tailSeconds, setTailSeconds] = useState(2);
  const count = sampleCount({ keys, velocities });
  const tooMany = count > MAX_CONVERT_SAMPLES;
  const blocked = tooMany || count === 0;
  const confirm = () => {
    if (blocked) return;
    close();
    void convertToSampler(transport, device, { keys, velocities, preRollSeconds, warmUpNote: warmUpNote && preRollSeconds > 0, noteSeconds, tailSeconds });
  };
  // A hand edit leaves the preset.
  const editKeys = (next: number[]) => {
    setKeys(next);
    setKeyPreset(CUSTOM);
  };
  const editVelocities = (next: number[]) => {
    setVelocities(next);
    setVelocityPreset(CUSTOM);
  };

  return (
    <Dialog
      open
      onClose={close}
      title={t("sampler.convert.title", { name: device.name })}
      className="eth-sampler-convert"
      testId="convert-to-sampler"
      footer={
        <>
          <Button size="sm" tone="ghost" onClick={close}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" tone="accent" onClick={confirm} disabled={blocked}>
            {t("sampler.convert.action")}
          </Button>
        </>
      }
    >
      <p className="eth-sampler-convert__note">{t("sampler.convert.intro")}</p>
      <section className="eth-sampler-convert__section">
        <div className="eth-sampler-convert__row">
          <span className="eth-sampler-convert__label">{t("sampler.convert.notes")}</span>
          <Select
            aria-label={t("sampler.convert.notes_preset")}
            value={keyPreset}
            onChange={(v) => {
              if (v === CUSTOM) return;
              setKeyPreset(v);
              // Over the picked keys' span (C1–C6 with fewer than two).
              setKeys(presetKeys(Number(v), keys));
            }}
            options={[
              ...KEY_PRESET_STEPS.map((n) => ({
                value: String(n),
                label:
                  n === 1 ? t("sampler.convert.key_step_all") : n === 12 ? t("sampler.convert.key_step_octave") : t("sampler.convert.key_step_n", { count: n }),
              })),
              { value: CUSTOM, label: t("sampler.convert.custom") },
            ]}
          />
          <span className="eth-sampler-convert__count">{t("sampler.convert.key_count", { count: keys.length })}</span>
        </div>
        <SampleKeyboard keys={keys} onChange={editKeys} />
        <p className="eth-sampler-convert__note">{t("sampler.convert.keyboard_hint")}</p>
      </section>
      <section className="eth-sampler-convert__section">
        <div className="eth-sampler-convert__row">
          <span className="eth-sampler-convert__label">{t("sampler.convert.velocities")}</span>
          <Select
            aria-label={t("sampler.convert.velocities_preset")}
            value={velocityPreset}
            onChange={(v) => {
              if (v === CUSTOM) return;
              setVelocityPreset(v);
              setVelocities(presetVelocities(Number(v)));
            }}
            options={[
              ...VELOCITY_PRESET_COUNTS.map((n) => ({ value: String(n), label: t("sampler.convert.velocity_step_n", { count: n }) })),
              { value: CUSTOM, label: t("sampler.convert.custom") },
            ]}
          />
          <VelocityList velocities={velocities} onChange={editVelocities} />
        </div>
        <p className="eth-sampler-convert__values" data-testid="convert-velocities">
          {sortedSet(velocities).join(" · ")}
        </p>
      </section>
      <div className="eth-sampler-convert__grid">
        <label className="eth-sampler-convert__field">
          <span className="eth-sampler-convert__hinted">
            {t("sampler.convert.pre_roll")}
            <Hint text={t("sampler.convert.pre_roll_hint")} />
          </span>
          <NumberField
            aria-label={t("sampler.convert.pre_roll")}
            value={preRollSeconds}
            onChange={setPreRollSeconds}
            min={0}
            max={2}
            step={0.01}
            precision={2}
            unit={t("sampler.convert.seconds")}
          />
        </label>
        <div className="eth-sampler-convert__field eth-sampler-convert__warm-up">
          <span className="eth-sampler-convert__hinted">
            <Toggle
              size="sm"
              checked={warmUpNote && preRollSeconds > 0}
              onChange={setWarmUpNote}
              disabled={preRollSeconds <= 0}
              label={t("sampler.convert.warm_up_note")}
            />
            <Hint text={t("sampler.convert.warm_up_note_hint")} />
          </span>
        </div>
        <label className="eth-sampler-convert__field">
          <span>{t("sampler.convert.note_length")}</span>
          <NumberField
            aria-label={t("sampler.convert.note_length")}
            value={noteSeconds}
            onChange={setNoteSeconds}
            min={0.1}
            max={60}
            step={0.5}
            precision={1}
            unit={t("sampler.convert.seconds")}
          />
        </label>
        <label className="eth-sampler-convert__field">
          <span>{t("sampler.convert.tail")}</span>
          <NumberField
            aria-label={t("sampler.convert.tail")}
            value={tailSeconds}
            onChange={setTailSeconds}
            min={0}
            max={60}
            step={0.5}
            precision={1}
            unit={t("sampler.convert.seconds")}
          />
        </label>
      </div>
      <p className="eth-sampler-convert__summary" data-too-many={blocked || undefined}>
        <AudioLines aria-hidden />
        {t("sampler.convert.summary", { keys: keys.length, velocities: sortedSet(velocities).length, count })}
      </p>
      {tooMany ? (
        <p className="eth-sampler-convert__error">{t("sampler.convert.too_many", { max: MAX_CONVERT_SAMPLES })}</p>
      ) : count === 0 ? (
        <p className="eth-sampler-convert__error">{t("sampler.convert.no_keys")}</p>
      ) : (
        <p className="eth-sampler-convert__note">{t("sampler.convert.nearest")}</p>
      )}
      <p className="eth-sampler-convert__note">{t("sampler.convert.replaces", { name: device.name })}</p>
    </Dialog>
  );
}

/** An info button whose tooltip explains a setting (focusable: the keyboard reads it too). */
function Hint({ text }: { text: string }) {
  return (
    <Tooltip content={<span className="eth-sampler-convert__hint-text">{text}</span>}>
      <button type="button" className="eth-sampler-convert__hint" aria-label={text}>
        <Info aria-hidden />
      </button>
    </Tooltip>
  );
}

/** "Configure…": a popover listing every rendered velocity, each editable or removable. */
function VelocityList({ velocities, onChange }: { velocities: number[]; onChange: (velocities: number[]) => void }) {
  const t = useT();
  const add = velocityToAdd(velocities);
  return (
    <Popover
      placement="bottom-end"
      aria-label={t("sampler.convert.velocities")}
      className="eth-sampler-convert__velocities"
      trigger={(p) => (
        <Button size="sm" tone="ghost" {...p}>
          {t("sampler.convert.configure")}
        </Button>
      )}
    >
      <ul className="eth-sampler-convert__velocity-list">
        {velocities.map((v, i) => (
          // By index: a row keeps its field while its value is edited.
          <li key={i}>
            <NumberField
              aria-label={t("sampler.convert.velocity_n", { index: i + 1 })}
              value={v}
              min={1}
              max={127}
              onChange={(value) => onChange(sortedSet(velocities.map((x, j) => (j === i ? value : x))))}
            />
            <IconButton
              size="sm"
              tone="ghost"
              label={t("sampler.convert.remove_velocity", { value: v })}
              icon={<X />}
              disabled={velocities.length <= 1}
              onClick={() => onChange(velocities.filter((_, j) => j !== i))}
            />
          </li>
        ))}
      </ul>
      <Button
        size="sm"
        tone="ghost"
        disabled={add === null || velocities.length >= MAX_CONVERT_VELOCITIES}
        onClick={() => add !== null && onChange(sortedSet([...velocities, add]))}
      >
        <Plus aria-hidden />
        {t("sampler.convert.add_velocity")}
      </Button>
      <p className="eth-sampler-convert__note">{t("sampler.convert.velocities_hint")}</p>
    </Popover>
  );
}
