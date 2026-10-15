// @vitest-environment node
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { ARGS_FILE, UI_DIR, capitalsIn, checkCatalogs, readCatalog, renderArgsFile, sourceFiles, unusedKeys } from "./tooling/check";
import { fromPo, toPo } from "./tooling/po";
import { formatFindings, scanFiles, scanSource } from "./tooling/scan";
import { LOCALIZED_PATHS } from "./localized";

describe("the shipped catalogs", () => {
  it("parse, have every key in every language with the same arguments, and follow the lowercase and typography rules", () => {
    expect(checkCatalogs().join("\n")).toBe("");
  });

  it("have no unused keys (every key is named as a string literal somewhere)", () => {
    expect(unusedKeys(Object.keys(readCatalog("en")))).toEqual([]);
  });

  it("have fresh argument types (`pnpm i18n gen`)", () => {
    expect(readFileSync(ARGS_FILE, "utf8")).toBe(renderArgsFile());
  });

  it("the localized paths have no hard-coded text left (the ratchet: docs/LOCALIZATION.md)", () => {
    const files = sourceFiles(LOCALIZED_PATHS.map((p) => join(UI_DIR, p))).filter((f) => !f.includes("/i18n/tooling/"));
    expect(files.length).toBeGreaterThan(0);
    expect(formatFindings(scanFiles(files)), "move this text to ui/src/i18n/locales/*.json (or mark it `// i18n-ignore`)").toBe("");
  });
});

describe("the catalog checks", () => {
  const check = (en: Record<string, string>, fr: Record<string, string>) => checkCatalogs({ en, fr });

  it("report bad ICU, missing and extra keys, and arguments that differ", () => {
    expect(check({ "a.x": "{n, plural, one {#}}" }, { "a.x": "{n, plural, one {#} other {#}}" })[0]).toMatch(/en: "a.x" has bad ICU syntax/);
    expect(check({ "a.x": "x", "a.y": "y" }, { "a.x": "x", "a.z": "z" })).toEqual([
      'fr: missing "a.y"',
      'fr: "a.z" isn\'t in en.json (removed or misspelled?)',
    ]);
    expect(check({ "a.x": "{name} ok" }, { "a.x": "{nom} ok" })).toEqual(['fr: "a.x" takes {nom:string | number}, en takes {name:string | number}']);
    // A plain argument may become a plural in the translation.
    expect(check({ "a.x": "{n} ch" }, { "a.x": "{n, plural, one {# canal} other {# canaux}}" })).toEqual([]);
  });

  it("enforce lowercase except proper nouns, in every language", () => {
    expect(check({ "a.x": "Open MIDI file" }, { "a.x": "ouvrir un fichier MIDI" })).toEqual([
      'en: "a.x" has capitals (Open): app text is lowercase (#524); proper nouns go in PROPER_NOUNS',
    ]);
    expect(capitalsIn("44.1 kHz, -6 dB, VST3 and AU, {Arg}")).toEqual(["Arg"]);
    // bpm and cpu are written lowercase (#815).
    expect(capitalsIn("120 BPM, 37% CPU")).toEqual(["BPM", "CPU"]);
    // Arguments and select keys aren't text.
    expect(check({ "a.x": "{Name} {k, select, A {a} other {b}}" }, { "a.x": "{Name} {k, select, A {a} other {b}}" })).toEqual([]);
  });

  it("enforce French typography: ’ and no-break spaces", () => {
    expect(check({ "a.x": "it's on: yes" }, { "a.x": "c'est activé : oui" })).toEqual([
      'fr: "a.x" uses \' : write the typographic apostrophe ’',
      'fr: "a.x" needs a no-break space (U+00A0, or U+202F before ; ! ?) before : ; ! ? % and inside « »',
    ]);
    expect(check({ "a.x": "on: yes" }, { "a.x": "c’est activé : oui" })).toEqual([]);
  });

  it("reject keys that aren't <area>.<name> snake_case", () => {
    expect(check({ Bad: "x" }, { Bad: "x" })[0]).toMatch(/key "Bad"/);
  });
});

describe("the hard-coded text scan", () => {
  const texts = (code: string) => scanSource("x.tsx", code).map((f) => f.text);
  it("finds JSX text, text attributes and props, notify messages and branches", () => {
    expect(
      texts(`
        const tabs = [{ id: "a", label: "General" }];
        notify("Info", \`Exported \${name}\`);
        <div title="Hello there" aria-label={on ? "Turn off" : "Turn on"} className="eth-x">
          Some text {count} {busy && "Loading…"} {"MIDI"} {t("a.b")}
        </div>;
      `),
    ).toEqual(["General", "Exported", "Hello there", "Turn off", "Turn on", "Some text", "Loading…"]);
  });
  it("skips numbers, units, acronyms and `// i18n-ignore`", () => {
    expect(texts(`<div>{n} dB · 44.1 kHz · VST3 <b title="Ethereal">x</b></div>`)).toEqual(["Ethereal"]);
    expect(texts(`<div>\n  <b title="Ethereal">x</b> {/* i18n-ignore */}\n</div>`)).toEqual([]);
    expect(texts(`// i18n-ignore\nconst a = { label: "Ethereal" };`)).toEqual([]);
  });
});

describe("the .po round trip", () => {
  it("keeps keys, ICU and escapes", () => {
    const en = { "a.x": 'say "hi"\\', "a.y": "{n, plural, one {# clip} other {# clips}}", "a.z": "z" };
    const fr = { "a.x": "dis « salut »", "a.y": "{n, plural, one {# clip} other {# clips}}" };
    const po = toPo(en, fr, "fr");
    expect(po).toContain('msgctxt "a.x"\nmsgid "say \\"hi\\"\\\\"');
    expect(fromPo(po)).toEqual(fr);
  });
});
