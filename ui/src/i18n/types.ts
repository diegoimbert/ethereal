import type { MessageFormatElement } from "@formatjs/icu-messageformat-parser";
import type EN from "./locales/en.json";
import type { MessageArgs } from "./args.gen";

/**
 * A message as shipped: a plain string when it has no ICU syntax (most of them), otherwise
 * its ICU AST, parsed at build time (`tooling/vitePlugin.ts`) so the app ships no parser.
 */
export type CompiledMessage = string | MessageFormatElement[];
export type CompiledCatalog = Readonly<Record<string, CompiledMessage>>;

/**
 * Message keys an extension adds (its own catalogs, `i18nCatalogPlugin({ extraDirs })`): it
 * augments this interface with its English catalog's type (docs/EXTENSIONS.md).
 */
// eslint-disable-next-line @typescript-eslint/no-empty-object-type
export interface ExtensionMessages {}

/** Every message key: the keys of the English catalog (the source of truth), and the extensions'. */
export type MessageKey = keyof typeof EN | keyof ExtensionMessages;

/** A rich-text tag's renderer (`<b>…</b>` in a message): gets the tag's content. */
export type Tag<T = unknown> = (chunks: Array<string | T>) => unknown;

/** The arguments `key` takes (`[]` when it takes none), from the generated `args.gen.ts`. */
export type ArgsOf<K extends MessageKey> = K extends keyof MessageArgs ? [args: MessageArgs[K]] : [args?: undefined];

/** A language the app ships (a catalog in `locales/`). */
export type ShippedLocale = "en" | "fr";
/** `en-XA`: the pseudo-locale (accented, longer, bracketed English) for finding hard-coded text. */
export type Locale = ShippedLocale | "en-XA";
