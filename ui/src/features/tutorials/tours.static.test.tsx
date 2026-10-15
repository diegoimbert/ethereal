// Static checks of the guided tours (docs/TUTORIALS.md "Tests"): the ways tours break in a
// refactor, caught without running them. A target renamed or removed, a selector a step waits
// on that no longer matches anything, a desktop-only part shown on the web, a step that can't
// be finished, the layer order. Fast (no rendering of the app); the full walkthrough is the
// opt-in `just e2e-tours`.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { isValidElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { zIndex } from "@/theme/tokens";
import { HOST_TARGETS, PLATFORM_ENVS } from "./targets";
import { EXAMPLES, TOURS } from "./tours";
import { textOf, type TourContext, type TourDef, type TourPlatform, type TourStep } from "./types";
import { visibleSteps } from "./store";

const HERE = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(HERE, "../..");
const read = (abs: string) => readFileSync(abs, "utf8");

/** The UI's source files (tests and the tutorials feature itself left out). */
const sources: Array<{ path: string; text: string }> = (readdirSync(SRC, { recursive: true }) as string[])
  .filter((f) => /\.(tsx?|jsx?)$/.test(f) && !/\.(test|spec|bench)\.[jt]sx?$/.test(f))
  .map((f) => join(SRC, f))
  .filter((abs) => !abs.startsWith(HERE))
  .map((abs) => ({ path: relative(SRC, abs), text: read(abs) }));

/** The tour definitions' source (the selectors their functions query). */
const tourSources = (readdirSync(join(HERE, "tours")) as string[])
  .filter((f) => f.endsWith(".tsx") || f.endsWith(".ts"))
  .map((f) => ({ path: f, text: read(join(HERE, "tours", f)) }));

/**
 * Every `data-tour` name the UI renders: `data-tour="…"`, and the `tour` props components pass
 * on to it (`<Field tour="…">`, `<SubMenu tour="…">`, `Tabs` items `{ tour: … }`). A template
 * (`settings-tab-${tab.id}`) becomes a pattern.
 */
function declaredTargets(): { names: Set<string>; patterns: RegExp[] } {
  const names = new Set<string>();
  const patterns: RegExp[] = [];
  const re = /(?:\bdata-tour=\{?|\btour=\{?|\btour:\s*)(["'`])([^"'`]+)\1/g;
  for (const { text } of sources)
    for (const m of text.matchAll(re)) {
      const v = m[2]!;
      if (m[1] === "`" && v.includes("${")) {
        const esc = v.split(/\$\{[^}]*\}/).map((p) => p.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
        patterns.push(new RegExp(`^${esc.join("[\\w-]+")}$`));
      } else names.add(v);
    }
  return { names, patterns };
}
const DECLARED = declaredTargets();
const exists = (name: string) => DECLARED.names.has(name) || DECLARED.patterns.some((p) => p.test(name));

const ctxOf = (p: TourPlatform): TourContext => ({ transport: null, env: PLATFORM_ENVS[p] });
const tours = Object.values(TOURS);
const where = (tour: TourDef, step: TourStep) => `tour "${tour.id}", step "${step.id}"`;

/** The visible text of a bubble's content (title, body or fallback), for a host. */
const plain = (node: unknown) => (isValidElement(node) ? renderToStaticMarkup(node).replace(/<[^>]+>/g, "") : String(node ?? ""));

describe("tour definitions", () => {
  it("found the tours and the data-tour names (sanity)", () => {
    expect(tours.length).toBeGreaterThanOrEqual(3);
    expect(DECLARED.names.size).toBeGreaterThan(10);
  });

  it("every step's target is a data-tour name the UI renders", () => {
    const missing: string[] = [];
    for (const tour of tours)
      for (const step of tour.steps)
        if (Array.isArray(step.targets)) for (const name of step.targets) if (!exists(name)) missing.push(`${where(tour, step)}: data-tour="${name}"`);
    expect(missing, "targets with no data-tour attribute in ui/src (renamed or removed?)").toEqual([]);
  });

  it("every selector a tour queries (targets found by code, waiting conditions) matches the UI's markup", () => {
    const missing: string[] = [];
    for (const { path, text } of tourSources) {
      // Attribute selectors in string literals: [data-tour="…"], [aria-label="…"], [role="…"].
      for (const m of text.matchAll(/\[([\w-]+)="([^"$]+)"\]/g)) {
        const [, attr, value] = m as unknown as [string, string, string];
        if (attr === "data-tour") {
          if (!exists(value)) missing.push(`tours/${path}: [data-tour="${value}"]`);
          continue;
        }
        // The attribute set literally (`aria-label="Project"`, `role="menu"`): an i18n or a
        // copy sweep that turns it into t(…) or another word breaks the tour.
        const literal = new RegExp(`\\b${attr}=\\{?["'\`]${value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}["'\`]`);
        if (!sources.some((s) => literal.test(s.text))) missing.push(`tours/${path}: [${attr}="${value}"]`);
      }
    }
    expect(missing, "selectors in the tours that no source renders literally").toEqual([]);
  });

  it("every tour and step has its fields: ids, titles, bodies, hints on waiting steps", () => {
    const problems: string[] = [];
    const tourIds = new Set<string>();
    for (const tour of tours) {
      if (tourIds.has(tour.id)) problems.push(`tour id "${tour.id}" twice`);
      tourIds.add(tour.id);
      if (TOURS[tour.id] !== tour) problems.push(`tour "${tour.id}" listed under another key in tours/index.ts`);
      if (!Number.isInteger(tour.version) || tour.version < 1) problems.push(`tour "${tour.id}": version ${tour.version}`);
      if (!tour.title.trim() || !tour.projectName.trim()) problems.push(`tour "${tour.id}": title / projectName empty`);
      if (tour.platforms.length === 0) problems.push(`tour "${tour.id}": no platforms`);
      if (tour.steps.length === 0) problems.push(`tour "${tour.id}": no steps`);
      const ids = new Set<string>();
      for (const step of tour.steps) {
        const at = where(tour, step);
        if (!/^[a-z0-9-]+$/.test(step.id)) problems.push(`${at}: id is not kebab-case`);
        if (ids.has(step.id)) problems.push(`${at}: id used twice`);
        ids.add(step.id);
        if (!step.title.trim()) problems.push(`${at}: no title`);
        if (step.body === undefined || step.body === null || step.body === "") problems.push(`${at}: no body`);
        if (Array.isArray(step.targets) && step.targets.length === 0) problems.push(`${at}: empty targets (omit them for a centered bubble)`);
        if (step.until) {
          // A waiting step shows no "next": the user must see what to do, and where.
          if (!step.hint?.trim()) problems.push(`${at}: waits (until) without a hint`);
          if (!step.targets) problems.push(`${at}: waits (until) without a target to act on`);
        }
        if (step.interactive && !step.targets) problems.push(`${at}: interactive without targets`);
        if (step.hint && !step.until) problems.push(`${at}: a hint without until (it is only shown while waiting)`);
        for (const a of step.actions ?? []) if (!a.id || !a.label.trim()) problems.push(`${at}: an action without id / label`);
      }
    }
    expect(problems).toEqual([]);
  });

  it("every tour is offered on the welcome screen", () => {
    const getStarted = read(join(SRC, "features/project/GetStarted.tsx"));
    // The example beats' tours are offered by the examples section (Examples.tsx lists EXAMPLES).
    const examples = new Set(EXAMPLES.map((e) => e.tour));
    const unlisted = tours.filter((t) => !examples.has(t) && !new RegExp(`tour:\\s*${exportName(t)}\\b`).test(getStarted)).map((t) => t.id);
    expect(unlisted, "tours missing from GetStarted.tsx's ARTICLES").toEqual([]);
    expect(read(join(SRC, "features/project/Examples.tsx"))).toMatch(/EXAMPLES\.map\(/);
    expect(read(join(SRC, "features/project/ProjectScreen.tsx"))).toMatch(/<Examples\b/);
    // Every example's tour is listed (TOURS), and makes the example's own project.
    for (const e of EXAMPLES) {
      expect(TOURS[e.tour.id], e.slug).toBe(e.tour);
      expect(e.tour.project, e.slug).toEqual({ template: e.template });
    }
  });

  it("renders every step's text on every platform it runs on, without overflowing the bubble", () => {
    const problems: string[] = [];
    for (const tour of tours)
      for (const p of tour.platforms) {
        const ctx = ctxOf(p);
        for (const step of visibleSteps(tour, ctx)) {
          const at = `${where(tour, step)} (${p})`;
          for (const [what, t] of [
            ["body", step.body],
            ["fallback", step.fallback],
          ] as const) {
            if (t === undefined) continue;
            let text: string;
            try {
              text = plain(textOf(t, ctx));
            } catch (e) {
              problems.push(`${at}: ${what} throws: ${String(e)}`);
              continue;
            }
            if (!text.trim()) problems.push(`${at}: ${what} renders empty`);
            // The bubble is 380px wide and scrolls past the window's height: past ~900
            // characters a step is a wall of text on a laptop (split it).
            if (text.length > 900) problems.push(`${at}: ${what} is ${text.length} characters (max 900)`);
          }
          if (step.illustration !== undefined) {
            try {
              renderToStaticMarkup(<>{step.illustration}</>);
            } catch (e) {
              problems.push(`${at}: illustration throws: ${String(e)}`);
            }
          }
        }
      }
    expect(problems).toEqual([]);
  });

  it("waiting conditions and targets found by code run on a fresh app without throwing", () => {
    const problems: string[] = [];
    for (const tour of tours)
      for (const p of tour.platforms)
        for (const step of visibleSteps(tour, ctxOf(p))) {
          try {
            const v = step.until?.(ctxOf(p));
            if (v !== undefined && typeof v !== "boolean") problems.push(`${where(tour, step)}: until returns ${typeof v}`);
            if (typeof step.targets === "function" && !Array.isArray(step.targets(ctxOf(p)))) problems.push(`${where(tour, step)}: targets() is not a list`);
          } catch (e) {
            problems.push(`${where(tour, step)} (${p}): ${String(e)}`);
          }
        }
    expect(problems).toEqual([]);
  });
});

describe("tour platforms", () => {
  it("no step shows a part of the UI the platform doesn't have, unless it has a fallback", () => {
    const problems: string[] = [];
    for (const tour of tours)
      for (const p of tour.platforms) {
        const env = PLATFORM_ENVS[p];
        for (const step of visibleSteps(tour, ctxOf(p))) {
          if (!Array.isArray(step.targets)) continue;
          const here = step.targets.filter((name) => HOST_TARGETS[name]?.(env) ?? true);
          if (here.length === 0 && step.fallback === undefined)
            problems.push(`${where(tour, step)} on ${p}: its targets (${step.targets.join(", ")}) don't exist there: guard it with \`when\``);
          // A waiting step whose only targets are missing would show "we'll skip it".
          if (here.length === 0 && step.until) problems.push(`${where(tour, step)} on ${p}: waits on targets that don't exist there`);
        }
      }
    expect(problems).toEqual([]);
  });

  it("every tour shows steps on each platform it declares, ending on a step with next / finish", () => {
    for (const tour of tours)
      for (const p of tour.platforms) {
        const shown = visibleSteps(tour, ctxOf(p));
        expect(shown.length, `tour "${tour.id}" on ${p}`).toBeGreaterThan(1);
        expect(shown.at(-1)!.until, `tour "${tour.id}" on ${p}: the last step waits`).toBeUndefined();
      }
  });

  it("HOST_TARGETS only names data-tour names the UI renders", () => {
    expect(Object.keys(HOST_TARGETS).filter((n) => !exists(n))).toEqual([]);
  });

  it("a tour offered on mobile needs the mobile shell to mount the tour overlay", () => {
    const shell = read(join(SRC, "features/mobile/MobileShell.tsx"));
    const mounted = /<TourRoot\b/.test(shell);
    const mobile = tours.filter((t) => t.platforms.includes("mobile")).map((t) => t.id);
    if (!mounted) expect(mobile, "tours declaring mobile, but MobileShell has no <TourRoot />").toEqual([]);
  });
});

describe("tour layers (#753, #754)", () => {
  it("dim < menus and popovers < highlight ring < bubble < tooltips, dim above dialogs", () => {
    const tour = read(join(HERE, "tutorials.css"));
    const kit = read(join(SRC, "kit/kit.css"));
    const z = (css: string, selector: string) => {
      const esc = selector.replace(/[.-]/g, (c) => `\\${c}`);
      const m = new RegExp(`^${esc}\\s*\\{[^}]*z-index:\\s*var\\(--eth-z-([a-z-]+)\\)`, "m").exec(css);
      expect(m, `${selector} has a z-index token`).not.toBeNull();
      return Number(zIndex[m![1]!.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase()) as keyof typeof zIndex]);
    };
    const order = [
      Number(zIndex.dialog),
      z(tour, ".eth-tour"),
      z(kit, ".eth-popover"),
      z(tour, ".eth-tour__rings"),
      z(tour, ".eth-tour__bubble"),
      Number(zIndex.tooltip),
    ];
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(new Set(order).size).toBe(order.length);
  });
});

/** The exported constant name of a tour (`JAM_TOUR`), from tours/index.ts. */
function exportName(tour: TourDef): string {
  const index = read(join(HERE, "tours/index.ts"));
  const m = new RegExp(`\\[(\\w+)\\.id\\]:\\s*\\1`, "g");
  for (const [, name] of index.matchAll(m)) {
    for (const { text } of tourSources) if (new RegExp(`export const ${name}\\b[^=]*=\\s*\\{\\s*id:\\s*"${tour.id}"`).test(text)) return name!;
  }
  return `<no export for ${tour.id}>`;
}
