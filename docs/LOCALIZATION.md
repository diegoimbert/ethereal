# Localization

The UI ships in English and French. Everything here is built so that a language is one
catalog file plus one line of code.

## How it works

- **Catalogs**: `ui/src/i18n/locales/<code>.json`, flat `key → ICU MessageFormat message`.
  `en.json` is the source of truth: the keys and their arguments come from it.
- **Runtime**: FormatJS [`intl-messageformat`](https://formatjs.github.io/docs/intl-messageformat)
  plus a thin wrapper in `ui/src/i18n/`. The catalogs are compiled to the ICU AST at build
  time (`i18n/tooling/vitePlugin.ts`, the `virtual:eth-locale/<code>` modules), so the app
  ships no message parser (about 3 KB gzipped of runtime). English is in the main bundle;
  every other language is a lazy chunk, loaded before the first render when it's the user's
  language (`initI18n()` in the entries) or on a switch.
- **Why this stack**: the measured options (minified + gzip, runtime only) were i18next +
  i18next-icu + react-i18next 32 KB (24 KB precompiled), react-intl 15 KB (8 KB precompiled),
  Lingui core 2 KB, intl-messageformat precompiled 3 KB. Lingui is as small but its value is
  a compile-time macro (a Babel/SWC pass in dev, build and vitest, which the oxc-based Vite 8
  toolchain doesn't run) and source text as keys. intl-messageformat is the reference ICU
  implementation on the platform `Intl` APIs, needs no transform, and works the same in
  components, stores and menus built as data.
- **Language choice**: the system language at first run (`navigator.languages`: any `fr*`
  → French, else English), then Settings → general → language. Kept on this device
  (localStorage `eth-locale`, like the theme), never in the project. It applies live: no
  reload. `?locale=fr` overrides it for one page load (tests, screenshots). `<html lang>`
  follows.
- **Desktop**: the macOS menu bar takes its text from the UI (`menu.*` keys;
  `features/desktop-menu` sends `menu_set_labels` at start and on every switch, and
  `app_menu.rs` rebuilds the menu, keeping the keymap's key equivalents).
  Its built-in defaults, shown until the UI loads, are the English catalog's (a Rust test
  checks it). The native file dialogs' titles and filters go through `t` too.
- **Docs** (`/docs`): the shell (search, navigation, pager, callouts, the shortcut tables'
  headers) is localized and follows the app's language. The pages are Markdown per language:
  `docs-site/content-fr/<section>/<page>.md` translates `docs-site/content/<section>/<page>.md`
  (a lazy chunk per language, `features/docs/contentFr.ts`, registered in `content.ts`
  `TRANSLATIONS`). A translated page replaces the English one; a page not translated yet shows
  in English with a notice ("this page isn't translated yet"). A translation keeps the English
  page's front-matter except `title` and `summary`, the same headings in the same order, each
  ending with the English heading's id (`## trois façons de l’utiliser {#three-ways-to-run-it}`,
  so links and the coverage anchors still land), and the same shortcuts and link targets.
  UI labels it quotes are the catalog's (`fr.json`): a label the English page quotes in bold is
  quoted in bold as its `fr.json` translation (`NOT_LABELS` lists the bold words that aren't
  labels there). It follows the French typography below.
  `features/docs/translations.test.ts` checks all of that. When an English page gains, loses
  or renames a heading, gains or loses a paragraph, list item, table row or link (or its
  `covers` change), its translation is stale: the app shows that
  page in English until it is updated, and the test fails, so a PR that changes an English page
  updates `content-fr` too (as a new UI string gets its `fr.json` entry). Adding a language's docs: a
  `contentXx.ts` like `contentFr.ts`, one line in `TRANSLATIONS`, one in the test.

## Writing UI text

```tsx
import { useT, Trans } from "@/i18n";

function Panel({ count }: { count: number }) {
  const t = useT(); // re-renders this component on a language switch
  return (
    <>
      <h3>{t("settings.audio.input_check")}</h3>
      <p>{t("settings.audio.dropouts", { count })}</p>
      <Trans k="settings.relay.in_session" values={{ session: <strong>{name}</strong> }} />
    </>
  );
}
```

- In a component, use `useT()`. Outside one (a store, a menu built as data, a Tauri dialog),
  import `t` and call it when the text is shown, never at module load (it would freeze the
  language of the moment). Lists of options keep message keys (`label: "settings.tabs.audio"`
  typed `MessageKey`) and call `t` at render.
- Keys are `<area>.<surface>.<name>` in lowercase snake_case (`settings.audio.buffer_size`,
  `kit.select.no_matches`). Words used everywhere live under `common.*`. Never build a key at
  runtime (`` t(`x.${id}`) ``): write the map of literal keys, which keeps the unused-key
  check exact.
- Keys and arguments are type-checked: `t("settings.audio.dropouts")` without `{ count }` is
  a type error. After editing `en.json`, run `pnpm i18n gen` (it rewrites
  `ui/src/i18n/args.gen.ts`).
- Use ICU for anything that varies: plurals (`{count, plural, one {# clip} other {# clips}}`),
  choices (`{platform, select, mac {macOS} other {this platform}}`), and full sentences with
  arguments in them, never concatenated fragments (word order differs between languages).
- Text that isn't translatable (a program name, a brand in a sample) goes in a constant, or
  gets `// i18n-ignore` on its line.
- User content (track, clip, device, preset, file and people names) is never translated and
  carries `.eth-user-text`.

### Lowercase (#524)

All app text is lowercase in every language, French included (the brand style), except the
proper nouns and acronyms listed in `i18n/tooling/text.ts` (`MIDI`, `VST3`, `CLAP`, `AU`,
`BPM`, `dB`, `Hz`, `macOS`…). The catalog test enforces it per language. No stylesheet changes
the case: the strings themselves are lowercase, and `ui/src/theme/uiCase.test.ts` checks the
literals still in the source (and that no CSS uses `text-transform`).

### Numbers, dates, sorting

`ui/src/i18n/format.ts`, all following the current language:

- `formatFixed(value, digits)`: what an editable field shows (`1.5` / `1,5`, no grouping,
  never `-0`). `NumberField` uses it.
- `parseNumber(text)`: what a field reads. `1.5` and `1,5` both mean 1.5 in every language (a
  French numpad types `.`); `1 000,5` and `1,000.5` are grouped; a lone `,` followed by three
  digits is a thousands separator in English only.
- `formatNumber`, `formatDate`, `formatDateTime`, `formatRelativeTime`, `formatList`.
- `compareText` (`Intl.Collator`, numeric: "track 2" before "track 10", accents in their
  place) for sorted lists; `foldForSearch` (case and accents ignored) for search fields.
- `inEnglish(fn)` runs `fn` with the English catalog: the command palette and the shortcut
  editor match a command by its name in the app's language and by its English name.
- Units stay as they are (`dB`, `Hz`, `ms`, `BPM`, `%`); French puts a no-break space before
  them (`48 000 Hz`, `20 %`).

### Text from the engine

The engine sends a `TextKey` (`{ id: "undo.clip.move", args: { count: 3 } }`, `ether_model::text`)
next to its English text, and the UI shows `tText(key, english)`: the message in the user's
language, or the English when there is no key or this build doesn't know the id (a newer
engine). Arguments can be numbers, names (user content, shown as is) or nested keys ("AI:
{edit}" wraps the edit's own label).

- **Undo labels** (`undo.*`): every command's label (`crates/ether-controller/src/doc/labels.rs`)
  is a message; history steps carry `label_key`, `HistoryState` carries `undo_key` /
  `redo_key`. The English comes from the same catalog: `ether-controller`'s build.rs compiles
  the `undo.*` entries of `en.json` in, and `crate::text` formats them (the small ICU subset
  they use), so agents, MCP and logs read exactly the English the UI shows. A test builds
  every command and checks its message exists and formats to its label.
- A UI `Batch` may send its own `key`; a `Batch` label that is one of the catalog's messages
  without arguments is keyed by the engine.
- The unused-key check also reads the controller's and the desktop shell's Rust sources, so
  write message ids as full literals there (`doc::undo("undo.capture.capture")`).
- **Notifications** (`notice.*`): `Event::Notification` carries `key` next to its English
  `message`. In the controller, `notify(out, level, msg("notice.…").arg(…))`; text that
  comes from deeper layers (a decoder's error, a plugin's message) stays an English
  argument (`{error}`). The native and web hosts' audio notices (`notice.audio.*`) work the
  same way, through `ether_controller::text`.
- **Errors** (`error.*`): `CommandError` carries an optional `key`. The errors a person
  meets in the UI are keyed (`crate::tx::keyed_err(code, msg("error.…"))`: no project open,
  nothing to undo, a frozen track, the project file errors, the sharing and session states…);
  the ones only agents and scripts meet (malformed arguments, unknown ids) stay English.
  The UI shows `commandErrorText(error)` (`@/transport`), which `errorMessage` uses.
  An error inside another message goes as a nested key (`crate::text::error_arg`).
- `crate::text`'s tests check that every `notice.*` / `error.*` id in the controller and the
  hosts exists in the catalog.
- **Built-in device text** (`engine.*`): the names of the built-in devices and modulators,
  their parameters, groups and choices, and their panels' section titles. The engine keeps
  its English ("Attack Time", "Low-pass 24"; agents and MCP read it); the UI shows each one
  through the catalog, so lowercase English or the user's language. `ui/src/i18n/engine-text.json`
  lists every string (`crates/ether-devices/tests/engine_text.rs` fails when it's stale:
  `UPDATE_ENGINE_TEXT=1 cargo test -p ether-devices --test engine_text`), and `pnpm i18n gen`
  gives each one a key (`engineText.gen.ts`, numbered names share one: "Step 12" is
  `engine.step_n` with `{n}`) and adds new ones to `en.json` in lowercase; translate them like
  any other key. `localizeDescriptor` / `localizeModulator` (`@/i18n`) give a built-in's
  descriptor in the current language (`useDescriptor`, `useBuiltinTypes`, `useModulatorKinds`
  and the automation targets already do); plugins keep their own text. Code that matches a
  choice or a parameter by its English name reads `sourceParam(info)`. Undo labels name a
  built-in's parameter or device with the same key (`crate::text::engine_arg`), so "set cutoff"
  reads « régler coupure ».
- **Default names** (`name.*`): the engine names the tracks, devices, rack chains and markers it
  creates in the UI's language ("2 groupe", « réverbération », « chaîne 3 », the starter
  project's « synthé »). The UI sends `Settings::SetLanguage` at start and on every switch
  (`app/EngineLanguage.tsx`); `ether-controller`'s build.rs compiles the `name.*` and
  `engine.*` entries of every catalog in (`crate::text::default_name`,
  `device_default_name`). English device names stay lowercase as they were ("eq"). Once
  created, a name is the user's (never translated again); `state/defaultNames.ts` gives the
  same names for a field cleared back to its default.
- **Import and export reports** (other DAWs): their lines stay the engine's English, written
  in the app's lowercase style at the source (`crates/ether-import`, `crates/ether-export`:
  a line starts lowercase unless with an acronym or a product name, "bpm", "cpu"). They are
  technical and many (about 300 templates with their values), so they aren't in the catalogs
  yet; the report's own chrome (areas, headings, counts) is.

## Checks

`pnpm i18n check` (and `ui/src/i18n/catalogs.test.ts`, so `pnpm test` and `just check-all`):

- every message parses (ICU syntax);
- every language has exactly the English keys, with the same arguments (a plain `{n}` may
  become a plural or a select in a translation);
- no unused key (each key appears as a string literal in `ui/src` or `apps/web/src`);
- `args.gen.ts` is fresh;
- lowercase, as above;
- French typography: `’` rather than `'` (which also avoids ICU's apostrophe quoting), and a
  no-break space (U+00A0, or the narrow U+202F before `; ! ?`) before `: ; ! ? %` and inside
  `« »`;
- **the ratchet**: the paths in `ui/src/i18n/localized.ts` contain no hard-coded UI text
  (JSX text, `title`/`aria-label`/`placeholder`/`label`… attributes and props, `notify(…)`
  messages, default text props). Each area's localization PR adds its paths.

At runtime, a key missing from a language shows the English text and warns once in dev.

### The pseudo-locale

`en-XA` is English with accented letters, about 35 % longer, wrapped in `⟦ ⟧`, built from
the English catalog at runtime. Pick it in Settings → general → language in a dev build
(`just dev-ui`), or open any build with `?locale=en-XA`. Any text without brackets didn't go
through the catalogs; a clipped `⟧` is an overflow.
`ui/src/i18n/pseudo.test.tsx` renders the localized surfaces in it and fails on any visible
text or `aria-label`/`title`/`placeholder` that isn't pseudo-translated.

### Coverage

`pnpm i18n report` lists the files with hard-coded text and the share already localized
(`report --all <dir>` lists every string of a directory). Developer-only surfaces (the kit
gallery, the mock transport: `DEV_ONLY_PATHS` in `i18n/localized.ts`) are left out.

## Adding a language

1. Copy `ui/src/i18n/locales/en.json` to `<code>.json` (`de.json`) and translate the values.
   Keep the keys, the `{arguments}` and the ICU keywords (`plural`, `one`, `other`, `select`)
   as they are; add the plural categories the language needs (`few`, `many`…, see the
   [CLDR plural rules](https://www.unicode.org/cldr/charts/latest/supplemental/language_plural_rules.html)).
2. Register it: one line in `LOCALES` (`ui/src/i18n/runtime.ts`, with its own name, e.g.
   `Deutsch`) and its code in `ShippedLocale` (`ui/src/i18n/types.ts`). `detectLocale`
   picks it up for users whose system is in that language.
3. Run `pnpm i18n check` and `pnpm --filter @ethereal/ui test src/i18n`. Add the language's
   typography rules to `checkCatalogs` if it has some (French has its own).
4. Look at the app in it (`?locale=<code>`), especially the top bar, menus, dialogs and
   settings tabs (the Playwright smoke `apps/web/e2e/i18n.spec.ts` checks French for
   overflow; extend it).

### Translators

The catalogs are "ICU JSON" (the FormatJS format), which Crowdin and Weblate import as is.
For tools that prefer gettext: `pnpm i18n po fr` writes `fr.po` (`msgctxt` = the key, `msgid`
= the English message, `msgstr` = the translation, ICU kept as is) and
`pnpm i18n po-import fr fr.po` merges a translated file back into `fr.json`.

Rules for translations: follow the glossary below; keep the lowercase style; a sentence stays
a sentence (no capitals after a period either); keep messages about as short as the English
(French runs 20 to 30 % longer: prefer the shorter wording on buttons and tabs).

## Glossary (en → fr)

Music-software French as Ableton Live and Logic Pro use it. Keep these consistent.

| English | French | notes |
|---|---|---|
| track | piste | audio track: piste audio; MIDI track: piste MIDI |
| clip | clip | m. |
| arrangement | arrangement | |
| session (view) | session | |
| scene | scène | |
| device | appareil | a device on a track (instrument or effect); "périphérique" is hardware only |
| audio device (hardware) | périphérique audio | |
| rack | rack | instrument rack: rack d’instruments; drum rack: drum rack |
| chain | chaîne | |
| plugin | plug-in | |
| preset | preset | |
| automation | automation | f.; automation lane: couloir d’automation |
| envelope | enveloppe | |
| sample (audio file) | échantillon | |
| sample rate | fréquence d’échantillonnage | |
| buffer size | taille du buffer | |
| loop | boucle | |
| fade | fondu | crossfade: fondu enchaîné |
| warp | warp | warp markers: marqueurs de warp |
| freeze / flatten | geler / aplatir | |
| bounce / render / export | rendu / exporter | |
| take / comping | prise / comping | |
| quantize | quantifier | |
| split / cut | scinder / couper | split a clip: scinder (Ableton); cut is the clipboard's couper |
| tempo, BPM | tempo, BPM | |
| time signature | signature rythmique | |
| bar / beat | mesure / temps | |
| playhead | tête de lecture | |
| locator / marker | marqueur | |
| arm (a track) | armer | |
| monitoring | monitoring | |
| send / return | envoi / retour | |
| mixer | mixeur | |
| volume / pan | volume / panoramique | |
| mute / solo | muet / solo | |
| undo / redo | annuler / rétablir | |
| settings | réglages | |
| share | partager | sharing: partage |
| host (of a session) | hôte | |
| dropout | coupure | |
| input / output | entrée / sortie | |

## RTL readiness

No right-to-left language ships yet; what one would need:

- `<html dir>` set from the language (the runtime already sets `lang`).
- The stylesheets mostly use physical properties: about 55 CSS files under `ui/src` use
  `left`/`right`, `margin-left/right`, `padding-left/right`, `border-left/right` or
  `text-align: left` (about 300 declarations), and only a handful use logical ones
  (`margin-inline-*`, `inset-inline-*`). Convert them to logical properties
  (`margin-inline-start`, `inset-inline-end`, `text-align: start`) area by area.
- Timelines stay left-to-right (time runs left to right in every DAW, RTL ones included):
  the arrangement, piano roll, automation lanes, the overview and the waveform canvases keep
  their geometry; only the chrome around them mirrors.
- Icons with a direction (arrows, chevrons for "next", the browser's tree carets) need
  mirroring; transport icons (play, rewind) don't.
- Canvas text (rulers, clip names) is drawn by hand and would need the bidi algorithm for
  mixed-direction names.

## Not localized (on purpose)

- MCP / AI agent tool descriptions and the agent's view of the project: English, the models
  read it.
- Logs and engine diagnostics (the engine log, crash reports).
- The telemetry admin dashboard (an internal tool).
