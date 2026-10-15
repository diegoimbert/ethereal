import { useMemo, useSyncExternalStore } from "react";
import { getLocale, subscribeLocale, t } from "./runtime";
import type { ArgsOf, Locale, MessageKey } from "./types";

/** The current language; re-renders the component when it changes. */
export function useLocale(): Locale {
  return useSyncExternalStore(subscribeLocale, getLocale, getLocale);
}

/**
 * `t` for components: re-renders on a language switch, and is a new function per language,
 * so memos that list it as a dependency recompute.
 */
export function useT(): typeof t {
  const locale = useLocale();
  // One function per language (`locale` keys the memo).
  const translate = useMemo(() => {
    const bound = <K extends MessageKey>(key: K, ...args: ArgsOf<K>) => t(key, ...args);
    return Object.assign(bound, { locale });
  }, [locale]);
  return translate;
}
