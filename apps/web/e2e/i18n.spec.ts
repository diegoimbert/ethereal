// #667: localization. Switching to French in Settings > general > language applies at once
// (no reload), survives a reload, and the French text fits (no clipped labels). French
// number fields read "1,5" and "1.5". The pseudo-locale (`?locale=en-XA`, ~35 % longer,
// bracketed) shows that the settings take longer text without overflow.
// No sleeps: every step waits on UI state.
import { expect, test, type Locator, type Page } from "@playwright/test";
import { launch, openOnLaunch, pickOption } from "./ui";

test.use({ viewport: { width: 1280, height: 800 } });

/** Elements in `scope` whose text is cut (wider than their box) and don't scroll on purpose. */
async function clipped(scope: Locator): Promise<string[]> {
  return scope.evaluate((root) => {
    const out: string[] = [];
    for (const el of root.querySelectorAll<HTMLElement>("button, [role=tab], h2, h3, label, .eth-settings__row > span, .eth-audio-settings__label")) {
      if (!el.offsetParent || el.closest("[class*=scroll], .eth-user-text")) continue;
      const style = getComputedStyle(el);
      if (style.overflowX === "auto" || style.overflowX === "scroll") continue;
      if (el.scrollWidth > el.clientWidth + 1) out.push(`${el.tagName.toLowerCase()} "${el.textContent?.trim()}" (${el.scrollWidth} > ${el.clientWidth})`);
    }
    return out;
  });
}

/** The settings dialog (opened with the top bar's gear when it isn't open), on tab `tab` (a name, or an index). */
async function openSettings(page: Page, tab: string | number): Promise<Locator> {
  const dialog = page.getByRole("dialog").filter({ has: page.getByRole("tablist") });
  // The top bar's gear, in whatever language the page is in.
  if (!(await dialog.isVisible())) await page.locator('[data-tour="settings-button"]').click();
  const target = typeof tab === "number" ? dialog.getByRole("tab").nth(tab) : dialog.getByRole("tab", { name: tab, exact: true });
  await target.click();
  await expect(target).toHaveAttribute("aria-selected", "true");
  return dialog;
}

test("switching to French applies live, persists, fits, and French number fields read both decimal separators", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page);
  // A marker that a reload would lose: the switch must not reload the page.
  await page.evaluate(() => ((window as unknown as { __noReload: boolean }).__noReload = true));

  let dialog = await openSettings(page, "general");
  const language = () => dialog.getByTestId("settings-language").getByRole("combobox");
  // By the element: its name changes with the language.
  await pickOption(dialog, language(), "Français");
  await expect(page.locator("html")).toHaveAttribute("lang", "fr");
  await expect(dialog.getByRole("tab", { name: "général", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("dialog", { name: "réglages" })).toBeVisible();
  await expect(dialog.getByRole("heading", { name: "démarrage" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "terminé" })).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __noReload?: boolean }).__noReload)).toBe(true);

  for (const tab of ["général", "apparence", "souris", "audio"]) {
    dialog = await openSettings(page, tab);
    expect(await clipped(dialog), `clipped text in réglages > ${tab}`).toEqual([]);
  }

  // Decimal comma: shown "1,00"; "1,5" and "1.25" both commit.
  dialog = await openSettings(page, "souris");
  const sensitivity = dialog.getByRole("spinbutton", { name: "sensibilité du zoom" });
  await expect(sensitivity).toHaveValue("1,00");
  for (const [typed, shown] of [
    ["1,5", "1,50"],
    ["1.25", "1,25"],
  ] as const) {
    await sensitivity.focus();
    await sensitivity.fill(typed);
    await sensitivity.press("Enter");
    await expect(sensitivity).toHaveValue(shown);
  }
  await dialog.getByRole("button", { name: "rétablir les réglages par défaut" }).click();
  await expect(sensitivity).toHaveValue("1,00");

  // The choice is kept on this device: a reload comes back in French.
  await page.reload();
  await openOnLaunch(page);
  await expect(page.locator("html")).toHaveAttribute("lang", "fr");
  dialog = await openSettings(page, "général");
  await expect(dialog.getByRole("combobox", { name: "langue", exact: true })).toContainText("Français");
  // And back to English, live.
  await pickOption(dialog, language(), "English");
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(dialog.getByRole("tab", { name: "general", exact: true })).toBeVisible();
});

test("the pseudo-locale's longer text fits the settings", async ({ page }) => {
  test.setTimeout(90_000);
  await page.addInitScript(() => localStorage.setItem("eth-locale", "en-XA"));
  await launch(page);
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  // General, appearance, audio, input (the localized tabs so far).
  for (const i of [0, 1, 2, 3]) {
    const dialog = await openSettings(page, i);
    await expect(dialog.getByRole("tab").nth(i)).toContainText("⟦");
    expect(await clipped(dialog), `clipped text in the pseudo-locale's tab ${i}`).toEqual([]);
  }
});

test("undo labels from the engine show in French", async ({ page }) => {
  test.setTimeout(90_000);
  await page.addInitScript(() => localStorage.setItem("eth-locale", "fr"));
  await launch(page);
  await expect(page.locator("html")).toHaveAttribute("lang", "fr");
  // The arrangement's own text is French too (its add-track menu).
  await page.getByRole("button", { name: "ajouter une piste", exact: true }).click();
  await page.getByRole("menuitem", { name: "créer une piste midi", exact: true }).click();
  // The engine sends `undo.track.create` {kind: midi}; the UI formats it in French.
  await expect(page.getByRole("button", { name: "annuler", exact: true })).toHaveAttribute("title", "annuler ajouter une piste midi");
  // The shell is French too: the top bar, the rail, and the palette finds commands by their French names.
  await expect(page.getByRole("button", { name: "réglages", exact: true })).toBeVisible();
  await expect(page.getByRole("toolbar", { name: "transport", exact: true }).getByRole("button", { name: "lecture", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "palette de commandes", exact: true }).click();
  await page.getByRole("combobox", { name: "chercher une commande" }).fill("metronome");
  await expect(page.getByRole("listbox", { name: "commandes" }).getByRole("option").first()).toContainText("activer le métronome");
});

test("the docs page opens in the app's language", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("eth-locale", "fr"));
  await page.goto("/docs");
  await expect(page.locator("html")).toHaveAttribute("lang", "fr");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("documentation d’ethereal");
  await expect(page.getByRole("combobox", { name: "rechercher dans la documentation" })).toBeVisible();
  await expect(page).toHaveTitle("ethereal - documentation");
  // A translated page, in French, with the English anchors.
  await page.goto("/docs/welcome/what-is-ethereal#the-open-on-desktop-band");
  await expect(page.getByRole("heading", { level: 1 })).toContainText("qu’est-ce qu’ethereal");
  await expect(page.locator("#the-open-on-desktop-band")).toContainText("le bandeau ouvrir dans l’application");
  await expect(page.getByTestId("docs-untranslated")).toHaveCount(0);
});
