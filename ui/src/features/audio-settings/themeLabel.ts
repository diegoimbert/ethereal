import { isBuiltinTheme, type BuiltinThemeId, type ThemeDefinition } from "@/theme";
import { useT, type MessageKey } from "@/i18n";

const BUILTIN_LABELS: Record<BuiltinThemeId, MessageKey> = {
  dark: "settings.appearance.theme_dark",
  obsidian: "settings.appearance.theme_obsidian",
  ember: "settings.appearance.theme_ember",
  tide: "settings.appearance.theme_tide",
  dusk: "settings.appearance.theme_dusk",
  light: "settings.appearance.theme_light",
};

/** What the picker shows for a theme: a built-in's translated name, else the user's name for it. */
export function useThemeLabel(): (def: ThemeDefinition) => string {
  const t = useT();
  return (def) => (isBuiltinTheme(def.id) ? t(BUILTIN_LABELS[def.id]) : def.name);
}
