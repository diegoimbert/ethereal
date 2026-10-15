import { useSyncExternalStore } from "react";
import { NumberField, Select, Toggle } from "@/kit";
import { getMotionLevel, setMotionLevel, subscribeMotionLevel, type MotionLevel } from "@/timeline/motion";
import {
  TRANSLUCENCY_DEFAULT,
  TRANSLUCENCY_MAX,
  ZOOM_MAX,
  ZOOM_MIN,
  useTheme,
  useColorBlobs,
  useGlow,
  useTranslucency,
  useTranslucencyAmount,
  useZoom,
  type ThemeName,
} from "@/theme";
import { useT, type MessageKey } from "@/i18n";

const MOTION_LEVELS: { value: MotionLevel; label: MessageKey }[] = [
  { value: "classic", label: "settings.appearance.motion_classic" },
  { value: "fast", label: "settings.appearance.motion_fast" },
  { value: "reduced", label: "settings.appearance.motion_reduced" },
];

const THEMES: { value: ThemeName; label: MessageKey }[] = [
  { value: "dark", label: "settings.appearance.theme_dark" },
  { value: "light", label: "settings.appearance.theme_light" },
];

/** Settings > Appearance: theme, interface zoom and translucency (kept on this device). */
export function AppearanceSettings() {
  const t = useT();
  const [theme, setTheme] = useTheme();
  const [zoom, setZoom] = useZoom();
  const [blobs, setBlobs] = useColorBlobs();
  const [glow, setGlowOn] = useGlow();
  const motion = useSyncExternalStore(subscribeMotionLevel, getMotionLevel, getMotionLevel);
  const [translucent, setTranslucent] = useTranslucency();
  const [amount, setAmount] = useTranslucencyAmount();
  return (
    <div className="eth-settings__stack" data-testid="settings-appearance">
      <section className="eth-settings__section" aria-label={t("settings.appearance.interface")}>
        <h3 className="eth-settings__heading">{t("settings.appearance.interface")}</h3>
        <div className="eth-settings__row">
          <span>{t("settings.appearance.theme")}</span>
          <Select
            size="sm"
            aria-label={t("settings.appearance.theme")}
            options={THEMES.map((o) => ({ value: o.value, label: t(o.label) }))}
            value={theme}
            onChange={setTheme}
          />
        </div>
        <div className="eth-settings__row">
          <span>{t("settings.appearance.zoom")}</span>
          <NumberField
            size="sm"
            className="eth-settings__number"
            aria-label={t("settings.appearance.zoom")}
            value={zoom}
            min={ZOOM_MIN}
            max={ZOOM_MAX}
            step={5}
            unit="%"
            onChange={setZoom}
          />
        </div>
        <p className="eth-settings__hint">{t("settings.appearance.zoom_hint")}</p>
        <div className="eth-settings__row">
          <span>{t("settings.appearance.animations")}</span>
          <Select
            size="sm"
            aria-label={t("settings.appearance.animations")}
            options={MOTION_LEVELS.map((o) => ({ value: o.value, label: t(o.label) }))}
            value={motion}
            onChange={setMotionLevel}
          />
        </div>
        <p className="eth-settings__hint">{t("settings.appearance.animations_hint")}</p>
        <Toggle size="sm" checked={blobs} onChange={setBlobs} label={t("settings.appearance.color_blobs")} />
        <p className="eth-settings__hint">{t("settings.appearance.color_blobs_hint")}</p>
        <Toggle size="sm" checked={glow} onChange={setGlowOn} label={t("settings.appearance.glow")} />
        <p className="eth-settings__hint">{t("settings.appearance.glow_hint")}</p>
      </section>
      <section className="eth-settings__section" aria-label={t("settings.appearance.translucency")}>
        <h3 className="eth-settings__heading">{t("settings.appearance.translucency")}</h3>
        <div className="eth-settings__row">
          <span>{t("settings.appearance.translucency_amount")}</span>
          <NumberField
            size="sm"
            className="eth-settings__number"
            aria-label={t("settings.appearance.translucency")}
            value={amount}
            min={0}
            max={TRANSLUCENCY_MAX}
            unit="%"
            disabled={!translucent}
            onChange={setAmount}
          />
        </div>
        <Toggle size="sm" checked={!translucent} onChange={(off) => setTranslucent(!off)} label={t("settings.appearance.translucency_disable")} />
        <p className="eth-settings__hint">{t("settings.appearance.translucency_hint", { pct: TRANSLUCENCY_DEFAULT })}</p>
      </section>
    </div>
  );
}
