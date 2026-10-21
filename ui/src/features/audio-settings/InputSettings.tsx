import type { ReactNode } from "react";
import { Button, NumberField, Select, Toggle } from "@/kit";
import {
  PLATFORM,
  SENSITIVITY_MAX,
  SENSITIVITY_MIN,
  useInputSettings,
  type MiddleButtonAction,
  type PlainWheel,
  type SideButtonAction,
  type ZoomModifier,
} from "@/timeline";
import { useT } from "@/i18n";
import { openKeymapEditor } from "@/features/keymap";
import { useAudioSettings } from "./store";

/**
 * Settings > Mouse & keyboard > Mouse & trackpad: how the mouse wheel and buttons drive zoom and scrolling in
 * every timeline (arrangement, piano roll, automation), value axes and wheel-adjusted
 * controls. Kept on this device (`useInputSettings`); defaults depend on the platform.
 */
export function InputSettings() {
  const t = useT();
  const mod = PLATFORM === "mac" ? t("settings.input.mod_cmd") : t("settings.input.mod_ctrl");
  const s = useInputSettings((st) => st.settings);
  const set = useInputSettings((st) => st.set);
  const reset = useInputSettings((st) => st.reset);
  return (
    <div className="eth-settings__stack eth-input-settings" data-testid="input-settings">
      <section className="eth-settings__section" aria-label={t("settings.input.wheel")}>
        <h3 className="eth-settings__heading">{t("settings.input.wheel")}</h3>
        <div className="eth-audio-settings__grid">
          <Field label={t("settings.input.zoom_sensitivity")}>
            <NumberField
              aria-label={t("settings.input.zoom_sensitivity")}
              value={s.zoomSensitivity}
              min={SENSITIVITY_MIN}
              max={SENSITIVITY_MAX}
              step={0.05}
              precision={2}
              unit="×"
              size="sm"
              onChange={(zoomSensitivity) => set({ zoomSensitivity })}
            />
          </Field>
          <Field label={t("settings.input.scroll_sensitivity")}>
            <NumberField
              aria-label={t("settings.input.scroll_sensitivity")}
              value={s.scrollSensitivity}
              min={SENSITIVITY_MIN}
              max={SENSITIVITY_MAX}
              step={0.05}
              precision={2}
              unit="×"
              size="sm"
              onChange={(scrollSensitivity) => set({ scrollSensitivity })}
            />
          </Field>
          <Field label={t("settings.input.plain_wheel")}>
            <Select<PlainWheel>
              aria-label={t("settings.input.plain_wheel")}
              value={s.plainWheel}
              options={[
                { value: "scroll", label: t("settings.input.plain_scrolls") },
                { value: "zoom", label: t("settings.input.plain_zooms") },
              ]}
              onChange={(plainWheel) => set({ plainWheel })}
            />
          </Field>
          <Field label={s.plainWheel === "zoom" ? t("settings.input.scroll_modifier") : t("settings.input.zoom_modifier")}>
            <Select<ZoomModifier>
              aria-label={t("settings.input.zoom_modifier")}
              value={s.zoomModifier}
              options={[
                { value: "ctrlCmd", label: PLATFORM === "mac" ? t("settings.input.mod_cmd_or_ctrl") : t("settings.input.mod_ctrl") },
                { value: "alt", label: PLATFORM === "mac" ? t("settings.input.mod_option") : t("settings.input.mod_alt") },
              ]}
              onChange={(zoomModifier) => set({ zoomModifier })}
            />
          </Field>
        </div>
        <div className="eth-input-settings__toggles">
          <Toggle size="sm" label={t("settings.input.invert_zoom")} checked={s.invertZoom} onChange={(invertZoom) => set({ invertZoom })} />
          <Toggle size="sm" label={t("settings.input.invert_y")} checked={s.invertScrollY} onChange={(invertScrollY) => set({ invertScrollY })} />
          <Toggle size="sm" label={t("settings.input.invert_x")} checked={s.invertScrollX} onChange={(invertScrollX) => set({ invertScrollX })} />
          <Toggle
            size="sm"
            label={t("settings.input.shift_horizontal")}
            checked={s.shiftScrollsHorizontally}
            onChange={(shiftScrollsHorizontally) => set({ shiftScrollsHorizontally })}
          />
        </div>
        <p className="eth-audio-settings__note">{t("settings.input.wheel_note", { mod })}</p>
      </section>

      <section className="eth-settings__section" aria-label={t("settings.input.mouse_buttons")}>
        <h3 className="eth-settings__heading">{t("settings.input.mouse_buttons")}</h3>
        <div className="eth-audio-settings__grid">
          <Field label={t("settings.input.middle_drag")}>
            <Select<MiddleButtonAction>
              aria-label={t("settings.input.middle_drag")}
              value={s.middleButton}
              options={[
                { value: "pan", label: t("settings.input.middle_pans") },
                { value: "off", label: t("common.off") },
              ]}
              onChange={(middleButton) => set({ middleButton })}
            />
          </Field>
          <Field label={t("settings.input.side_buttons")}>
            <Select<SideButtonAction>
              aria-label={t("settings.input.side_buttons")}
              value={s.sideButtons}
              options={[
                { value: "none", label: t("settings.input.side_nothing") },
                { value: "undoRedo", label: t("settings.input.side_undo_redo") },
              ]}
              onChange={(sideButtons) => set({ sideButtons })}
            />
          </Field>
        </div>
      </section>

      <div className="eth-input-settings__footer">
        <span className="eth-audio-settings__note">{t("settings.input.saved_note", { platform: PLATFORM })}</span>
        <Button size="sm" onClick={reset}>
          {t("settings.input.reset")}
        </Button>
      </div>
    </div>
  );
}

/** A labelled row (a plain container: the Select trigger is a button, which a <label> would click twice). */
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="eth-audio-settings__field">
      <span className="eth-audio-settings__label">{label}</span>
      {children}
    </div>
  );
}

/** Settings > Mouse & keyboard > Keyboard shortcuts: opens the shortcuts editor. */
export function ShortcutSettings() {
  const t = useT();
  return (
    <div className="eth-settings__stack">
      <section className="eth-settings__section" aria-label={t("settings.input.shortcuts")} data-testid="settings-shortcuts">
        <h3 className="eth-settings__heading">{t("settings.input.shortcuts")}</h3>
        <p className="eth-settings__hint">{t("settings.input.shortcuts_hint")}</p>
        <div>
          <Button
            size="sm"
            onClick={() => {
              useAudioSettings.getState().close();
              openKeymapEditor();
            }}
          >
            {t("settings.input.shortcuts_edit")}
          </Button>
        </div>
      </section>
    </div>
  );
}
