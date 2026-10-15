/**
 * The built-in devices' display text (docs/LOCALIZATION.md "Text from the engine"). The
 * engine names parameters, groups, choices, panel sections, devices and modulators in English
 * ("Attack Time", "Low-pass 24", "Step 12"); the UI shows them through the catalog
 * (`engine.*`): lowercase English or the user's language. `engine-text.json` lists every
 * string (a Rust test keeps it current) and `engineText.gen.ts` maps each one to its key.
 */
import type { DeviceDescriptor, ModulatorDescriptor, ParamInfo } from "@/generated";
import type { DeviceLayout } from "@/generated";
import { ENGINE_TEXT } from "./engineText.gen";
import { formatMessage, getLocale, subscribeLocale } from "./runtime";

/** `text` (a built-in device's, as the engine sends it) in the current language; other text as is. */
export function engineText(text: string): string {
  const entry = ENGINE_TEXT[text];
  if (entry === undefined) return text;
  const out = typeof entry === "string" ? formatMessage(entry) : formatMessage(entry[0], { n: entry[1] });
  return String(out);
}

/** Localized copy → the engine's `ParamInfo` (its English choice labels drive the curves). */
const sources = new WeakMap<ParamInfo, ParamInfo>();

/** The engine's own `ParamInfo` behind a localized one (itself otherwise). */
export function sourceParam(info: ParamInfo): ParamInfo {
  return sources.get(info) ?? info;
}

function localizeParam(info: ParamInfo): ParamInfo {
  const out: ParamInfo = {
    ...info,
    name: engineText(info.name),
    group: info.group === null ? null : engineText(info.group),
    labels: info.labels === null ? null : info.labels.map(engineText),
  };
  sources.set(out, info);
  return out;
}

function localizeLayout(layout: DeviceLayout | undefined): DeviceLayout | undefined {
  if (!layout) return layout;
  return {
    ...layout,
    sections: layout.sections.map((s) => ({
      ...s,
      title: s.title === null ? null : engineText(s.title),
      items: s.items.map((i) => (i.label === null ? i : { ...i, label: engineText(i.label) })),
    })),
  };
}

/** One copy per descriptor and language (descriptors are cached, so is this). */
let cache = new WeakMap<object, object>();
let cachedFor = getLocale();
subscribeLocale(() => {
  cache = new WeakMap();
  cachedFor = getLocale();
});

function cached<T extends object>(d: T, make: (d: T) => T): T {
  if (cachedFor !== getLocale()) {
    cache = new WeakMap();
    cachedFor = getLocale();
  }
  let out = cache.get(d) as T | undefined;
  if (!out) cache.set(d, (out = make(d)));
  return out;
}

/**
 * A built-in device's descriptor with its display text in the current language (plugins and
 * missing devices are returned as they are: their text is the plugin's). Choice labels keep
 * their English in `sourceParam(info).labels`.
 */
export function localizeDescriptor(d: DeviceDescriptor): DeviceDescriptor {
  if (d.device_type.type !== "Builtin") return d;
  return cached(d, (x) => ({ ...x, name: engineText(x.name), params: x.params.map(localizeParam), layout: localizeLayout(x.layout ?? undefined) }));
}

/** A modulator kind's descriptor in the current language (`localizeDescriptor`). */
export function localizeModulator(d: ModulatorDescriptor): ModulatorDescriptor {
  return cached(d, (x) => ({ ...x, name: engineText(x.name), params: x.params.map(localizeParam), layout: localizeLayout(x.layout ?? undefined) }));
}
