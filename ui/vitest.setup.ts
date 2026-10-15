import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach } from "vitest";
import { MOTION } from "@/timeline/motion";
import { domCaseFindings } from "@/theme/uiCaseDom";
import { appendFileSync } from "node:fs";

afterEach((ctx) => {
  // The UI's own text is lowercase (#524): the final DOM of every test, but user content.
  const findings = typeof document === "undefined" ? [] : domCaseFindings(document.body);
  if (process.env.ETH_CASE_LOG && findings.length)
    appendFileSync(process.env.ETH_CASE_LOG, JSON.stringify({ test: ctx.task.file?.name + " :: " + ctx.task.name, findings }) + "\n");
  cleanup();
});

// `waitFor` / `findBy*` give up after 1 s by default: wall time, which a loaded machine (the
// merge gate runs beside other gates and agents) can spend before the awaited update lands.
// A passing wait returns as soon as its condition holds; only a failure waits this long.
configure({ asyncUtilTimeout: 5_000 });

// Wheel/zoom animations apply instantly in tests (motion is tested on its own).
MOTION.enabled = false;

// jsdom has no ResizeObserver: an inert one (nothing is ever resized there), so views that
// observe their size (the arranger canvas) mount as in a browser.
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  } as unknown as typeof ResizeObserver;
}
