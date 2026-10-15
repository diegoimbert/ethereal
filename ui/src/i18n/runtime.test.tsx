import { act, render, screen } from "@testing-library/react";
import { useEffect } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { compareText, foldForSearch, formatFixed, parseNumber } from "./format";
import { PSEUDO_CLOSE, PSEUDO_OPEN, pseudoMessage } from "./pseudo";
import { useT } from "./hooks";
import { Trans } from "./Trans";
import { LOCALE_STORAGE_KEY, detectLocale, getLocale, inEnglish, initI18n, resetI18nForTests, setLocale, t, tText } from "./runtime";
import { compileMessage } from "./tooling/compile";

afterEach(() => {
  resetI18nForTests();
  localStorage.clear();
  document.documentElement.lang = "";
  vi.restoreAllMocks();
});

describe("t", () => {
  it("formats plain messages, arguments and plurals in the current language", async () => {
    expect(t("common.done")).toBe("done");
    expect(t("settings.audio.buffer_samples", { count: 1 })).toBe("1 sample");
    expect(t("settings.audio.buffer_samples", { count: 256 })).toBe("256 samples");
    await setLocale("fr");
    expect(getLocale()).toBe("fr");
    expect(document.documentElement.lang).toBe("fr");
    expect(t("common.done")).toBe("terminé");
    // French plurals: 0 and 1 are singular.
    expect(t("settings.audio.dropouts", { count: 0 })).toBe("0 coupure");
    expect(t("settings.audio.dropouts", { count: 2 })).toBe("2 coupures");
    expect(t("settings.input.saved_note", { platform: "linux" })).toBe("enregistré sur cet appareil. les réglages par défaut sont ceux de linux.");
    expect(localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("fr");
  });

  it("falls back to English (with a dev warning) for a key a language lacks", async () => {
    await setLocale("fr");
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    // A key the French catalog doesn't have (only possible mid-development: the catalog test forbids it).
    expect(t("no.such_key" as "common.done")).toBe("no.such_key");
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it("inEnglish formats in English for the duration of the call only", async () => {
    await setLocale("fr", { persist: false });
    expect(inEnglish(() => [t("common.done"), t("settings.audio.dropouts", { count: 2 })])).toEqual(["done", "2 dropouts"]);
    expect(t("common.done")).toBe("terminé");
    expect(getLocale()).toBe("fr");
    expect(() =>
      inEnglish(() => {
        throw new Error("x");
      }),
    ).toThrow("x");
    expect(t("settings.audio.dropouts", { count: 2 })).toBe("2 coupures");
  });

  it("drops a slower load overtaken by a newer switch", async () => {
    const slow = setLocale("fr");
    const fast = setLocale("en");
    await Promise.all([slow, fast]);
    expect(getLocale()).toBe("en");
  });
});

describe("engine messages (tText)", () => {
  it("formats the engine's message ids in the current language, nested ones included, else its English", async () => {
    const move = { id: "undo.clip.move", args: { count: 3 } };
    const ai = { id: "undo.agent.edit", args: { edit: move } };
    const remove = { id: "undo.device.remove", args: { device: "Reverb" } };
    expect(tText(move, "move 3 clips")).toBe("move 3 clips");
    await setLocale("fr");
    expect(tText(move, "move 3 clips")).toBe("déplacer 3 clips");
    expect(tText(ai, "AI: move 3 clips")).toBe("IA : déplacer 3 clips");
    // Names (user content) stay as they are.
    expect(tText(remove, "remove Reverb")).toBe("retirer Reverb");
    // No key, or one this build doesn't know (a newer engine): the English.
    expect(tText(undefined, "paste 3 clips")).toBe("paste 3 clips");
    expect(tText({ id: "undo.from_the_future", args: {} }, "warp time")).toBe("warp time");
  });
});

describe("startup language", () => {
  it("takes the saved choice, else the system's language, else English", async () => {
    expect(detectLocale(["de-DE", "fr-CA", "en"])).toBe("fr");
    expect(detectLocale(["de-DE"])).toBe("en");
    expect(detectLocale([])).toBe("en");
    localStorage.setItem(LOCALE_STORAGE_KEY, "fr");
    await initI18n();
    expect(getLocale()).toBe("fr");
  });
});

describe("components", () => {
  function Label() {
    const t = useT();
    return <span data-testid="label">{t("settings.title")}</span>;
  }

  it("re-render live on a language switch, without a remount", async () => {
    let mounts = 0;
    function Counted() {
      const t = useT();
      useEffect(() => {
        mounts++;
      }, []);
      return <span data-testid="label">{t("settings.title")}</span>;
    }
    render(
      <>
        <Label />
        <Counted />
      </>,
    );
    expect(screen.getAllByTestId("label").map((e) => e.textContent)).toEqual(["settings", "settings"]);
    await act(() => setLocale("fr"));
    expect(screen.getAllByTestId("label").map((e) => e.textContent)).toEqual(["réglages", "réglages"]);
    expect(mounts).toBe(1);
  });

  it("<Trans> puts elements into a message", () => {
    render(
      <p data-testid="p">
        <Trans k="project.actions.delete_named" values={{ name: <strong>jam</strong> }} />
      </p>,
    );
    expect(screen.getByTestId("p").innerHTML).toBe("delete “<strong>jam</strong>”?");
  });
});

describe("the pseudo-locale", () => {
  it("accents, lengthens and brackets text but keeps the ICU arguments working", async () => {
    const plain = pseudoMessage("open file");
    expect(plain).toMatch(new RegExp(`^${PSEUDO_OPEN}öþéñ ƒîļé·+${PSEUDO_CLOSE}$`));
    expect((plain as string).length).toBeGreaterThan("open file".length * 1.3);
    await setLocale("en-XA");
    expect(document.documentElement.lang).toBe("en");
    const s = t("settings.audio.buffer_samples", { count: 3 });
    expect(s.startsWith(PSEUDO_OPEN) && s.endsWith(PSEUDO_CLOSE)).toBe(true);
    expect(s).toContain("3 šáɱþļéš");
    expect(compileMessage("{n, plural, one {# x} other {# y}}")).not.toBe("");
  });
});

describe("formatting", () => {
  it("parses typed numbers with either decimal separator", () => {
    for (const [text, en, fr] of [
      ["1.5", 1.5, 1.5],
      ["1,5", 1.5, 1.5],
      [" -0,25 ", -0.25, -0.25],
      ["−3", -3, -3],
      ["1,000", 1000, 1],
      ["1,000.5", 1000.5, 1000.5],
      ["1 000,5", 1000.5, 1000.5],
      ["1 000,5", 1000.5, 1000.5],
      ["1.000.000", 1000000, 1000000],
      ["0,500", 0.5, 0.5],
      [".5", 0.5, 0.5],
      ["1e3", 1000, 1000],
      ["", null, null],
      ["abc", null, null],
      ["1,2,3", null, null],
      ["1.2.3", null, null],
    ] as const) {
      expect([text, parseNumber(text, "en")]).toEqual([text, en]);
      expect([text, parseNumber(text, "fr")]).toEqual([text, fr]);
    }
  });

  it("shows fixed decimals in the language's style, never -0", async () => {
    expect(formatFixed(1.5, 2)).toBe("1.50");
    expect(formatFixed(-0.001, 1)).toBe("0.0");
    expect(formatFixed(12345.6, 1)).toBe("12345.6");
    await setLocale("fr");
    expect(formatFixed(1.5, 2)).toBe("1,50");
    expect(formatFixed(-2.25, 1)).toBe("-2,3");
  });

  it("sorts and searches the way people read", () => {
    expect(["track 10", "track 2"].sort(compareText)).toEqual(["track 2", "track 10"]);
    expect(["zebra", "échantillon", "eau"].sort(compareText)).toEqual(["eau", "échantillon", "zebra"]);
    expect(foldForSearch("Échantillon Ça")).toBe("echantillon ca");
  });
});
