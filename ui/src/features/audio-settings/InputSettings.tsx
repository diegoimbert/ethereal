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

const PLATFORM_NAME = { mac: "macOS", windows: "Windows", linux: "Linux", other: "this platform" } as const;
const MOD = PLATFORM === "mac" ? "Cmd" : "Ctrl";

/**
 * Settings > Input (base-135): how the mouse wheel and buttons drive zoom and scrolling in
 * every timeline (arrangement, piano roll, automation), value axes and wheel-adjusted
 * controls. Kept on this device (`useInputSettings`); defaults depend on the platform.
 */
export function InputSettings() {
  const s = useInputSettings((st) => st.settings);
  const set = useInputSettings((st) => st.set);
  const reset = useInputSettings((st) => st.reset);
  return (
    <div className="eth-settings__stack eth-input-settings" data-testid="input-settings">
      <section className="eth-settings__section" aria-label="Wheel">
        <h3 className="eth-settings__heading">Wheel</h3>
        <div className="eth-audio-settings__grid">
          <Field label="Zoom sensitivity">
            <NumberField
              aria-label="Zoom sensitivity"
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
          <Field label="Scroll sensitivity">
            <NumberField
              aria-label="Scroll sensitivity"
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
          <Field label="Wheel without modifier">
            <Select<PlainWheel>
              aria-label="Wheel without modifier"
              value={s.plainWheel}
              options={[
                { value: "scroll", label: "Scrolls" },
                { value: "zoom", label: "Zooms" },
              ]}
              onChange={(plainWheel) => set({ plainWheel })}
            />
          </Field>
          <Field label={s.plainWheel === "zoom" ? "Scroll modifier" : "Zoom modifier"}>
            <Select<ZoomModifier>
              aria-label="Zoom modifier"
              value={s.zoomModifier}
              options={[
                { value: "ctrlCmd", label: PLATFORM === "mac" ? "Cmd (or Ctrl)" : "Ctrl" },
                { value: "alt", label: PLATFORM === "mac" ? "Option" : "Alt" },
              ]}
              onChange={(zoomModifier) => set({ zoomModifier })}
            />
          </Field>
        </div>
        <div className="eth-input-settings__toggles">
          <Toggle size="sm" label="Invert zoom direction" checked={s.invertZoom} onChange={(invertZoom) => set({ invertZoom })} />
          <Toggle
            size="sm"
            label="Invert vertical scrolling"
            checked={s.invertScrollY}
            onChange={(invertScrollY) => set({ invertScrollY })}
          />
          <Toggle
            size="sm"
            label="Invert horizontal scrolling"
            checked={s.invertScrollX}
            onChange={(invertScrollX) => set({ invertScrollX })}
          />
          <Toggle
            size="sm"
            label="Shift + wheel scrolls horizontally"
            checked={s.shiftScrollsHorizontally}
            onChange={(shiftScrollsHorizontally) => set({ shiftScrollsHorizontally })}
          />
        </div>
        <p className="eth-audio-settings__note">
          {MOD} + Shift + wheel zooms track and key heights. Trackpad pinch always zooms, never inverted. A mouse-wheel notch
          zooms by a fixed step on every platform.
        </p>
      </section>

      <section className="eth-settings__section" aria-label="Mouse buttons">
        <h3 className="eth-settings__heading">Mouse buttons</h3>
        <div className="eth-audio-settings__grid">
          <Field label="Middle-button drag">
            <Select<MiddleButtonAction>
              aria-label="Middle-button drag"
              value={s.middleButton}
              options={[
                { value: "pan", label: "Pans the view" },
                { value: "off", label: "Off" },
              ]}
              onChange={(middleButton) => set({ middleButton })}
            />
          </Field>
          <Field label="Back / Forward buttons">
            <Select<SideButtonAction>
              aria-label="Back / Forward buttons"
              value={s.sideButtons}
              options={[
                { value: "none", label: "Nothing" },
                { value: "undoRedo", label: "Undo / Redo" },
              ]}
              onChange={(sideButtons) => set({ sideButtons })}
            />
          </Field>
        </div>
      </section>

      <div className="eth-input-settings__footer">
        <span className="eth-audio-settings__note">Saved on this device. Defaults are for {PLATFORM_NAME[PLATFORM]}.</span>
        <Button size="sm" onClick={reset}>
          Reset to defaults
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
