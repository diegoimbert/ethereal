import { useState } from "react";
import { AudioLines } from "lucide-react";
import type { Device } from "@/generated";
import { Button, Dialog, NumberField, Select } from "@/kit";
import { useTransport } from "@/transport";
import { useT } from "@/i18n";
import { MAX_CONVERT_SAMPLES, convertToSampler, sampleCount, sampleNoteName, useSamplerConvert } from "./store";
import "./sampler-convert.css";

const KEY_STEPS = [1, 2, 3, 4, 5, 6, 8, 12] as const;
const VELOCITY_STEPS = [1, 2, 3, 4, 5, 6, 8, 12, 16] as const;

const DEFAULTS = {
  firstKey: 36,
  lastKey: 96,
  keyStep: 3,
  velocitySteps: 3,
  noteSeconds: 4,
  tailSeconds: 2,
};

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
  const [o, setO] = useState(DEFAULTS);
  const count = sampleCount(o);
  const tooMany = count > MAX_CONVERT_SAMPLES || count === 0;
  const keys = sampleCount({ ...o, velocitySteps: 1 });
  const set = <K extends keyof typeof o>(key: K, value: (typeof o)[K]) => setO((prev) => ({ ...prev, [key]: value }));
  const confirm = () => {
    if (tooMany) return;
    close();
    void convertToSampler(transport, device, o);
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
          <Button size="sm" tone="accent" onClick={confirm} disabled={tooMany}>
            {t("sampler.convert.action")}
          </Button>
        </>
      }
    >
      <p className="eth-sampler-convert__note">{t("sampler.convert.intro")}</p>
      <div className="eth-sampler-convert__grid">
        <label className="eth-sampler-convert__field">
          <span>{t("sampler.convert.first_key")}</span>
          <span className="eth-sampler-convert__key">
            <NumberField
              aria-label={t("sampler.convert.first_key")}
              value={o.firstKey}
              onChange={(v) => set("firstKey", Math.min(v, o.lastKey))}
              min={0}
              max={127}
            />
            <em>{sampleNoteName(o.firstKey)}</em>
          </span>
        </label>
        <label className="eth-sampler-convert__field">
          <span>{t("sampler.convert.last_key")}</span>
          <span className="eth-sampler-convert__key">
            <NumberField
              aria-label={t("sampler.convert.last_key")}
              value={o.lastKey}
              onChange={(v) => set("lastKey", Math.max(v, o.firstKey))}
              min={0}
              max={127}
            />
            <em>{sampleNoteName(o.lastKey)}</em>
          </span>
        </label>
        <label className="eth-sampler-convert__field">
          <span>{t("sampler.convert.key_step")}</span>
          <Select
            aria-label={t("sampler.convert.key_step")}
            value={String(o.keyStep)}
            onChange={(v) => set("keyStep", Number(v))}
            options={KEY_STEPS.map((n) => ({
              value: String(n),
              label: n === 1 ? t("sampler.convert.key_step_all") : t("sampler.convert.key_step_n", { count: n }),
            }))}
          />
        </label>
        <label className="eth-sampler-convert__field">
          <span>{t("sampler.convert.velocity_steps")}</span>
          <Select
            aria-label={t("sampler.convert.velocity_steps")}
            value={String(o.velocitySteps)}
            onChange={(v) => set("velocitySteps", Number(v))}
            options={VELOCITY_STEPS.map((n) => ({ value: String(n), label: t("sampler.convert.velocity_step_n", { count: n }) }))}
          />
        </label>
        <label className="eth-sampler-convert__field">
          <span>{t("sampler.convert.note_length")}</span>
          <NumberField
            aria-label={t("sampler.convert.note_length")}
            value={o.noteSeconds}
            onChange={(v) => set("noteSeconds", v)}
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
            value={o.tailSeconds}
            onChange={(v) => set("tailSeconds", v)}
            min={0}
            max={60}
            step={0.5}
            precision={1}
            unit={t("sampler.convert.seconds")}
          />
        </label>
      </div>
      <p className="eth-sampler-convert__summary" data-too-many={tooMany || undefined}>
        <AudioLines aria-hidden />
        {t("sampler.convert.summary", { keys, velocities: o.velocitySteps, count })}
      </p>
      {tooMany ? (
        <p className="eth-sampler-convert__error">{t("sampler.convert.too_many", { max: MAX_CONVERT_SAMPLES })}</p>
      ) : (
        <p className="eth-sampler-convert__note">{t("sampler.convert.nearest")}</p>
      )}
      <p className="eth-sampler-convert__note">{t("sampler.convert.replaces", { name: device.name })}</p>
    </Dialog>
  );
}
