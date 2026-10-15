import { useEffect } from "react";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { intlLocale, useLocale } from "@/i18n";
import { cmd } from "@/transport";

/**
 * Tells the engine the UI's language (`Settings::SetLanguage`, at start and on every switch):
 * it names the tracks, devices, rack chains and markers it creates in it (docs/LOCALIZATION.md).
 * An engine that doesn't know the command (an older remote one) keeps English.
 */
export function EngineLanguage(): null {
  const transport = useOptionalTransport();
  const locale = useLocale();
  useEffect(() => {
    if (!transport) return;
    transport.send(cmd("Settings", { type: "SetLanguage", language: intlLocale(locale) })).catch(() => undefined);
  }, [transport, locale]);
  return null;
}
