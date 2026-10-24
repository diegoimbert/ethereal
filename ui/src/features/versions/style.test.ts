// The recovery dialog keeps one height while projects are dismissed from it (no layout
// shift under the pointer when "dismiss" is clicked again and again).
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "versions.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

/** The declarations of the rules whose selector is exactly `selector`. */
const rule = (selector: string) =>
  [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)]
    .filter((m) => m[1]!.trim() === selector)
    .map((m) => m[2]!)
    .join(";");

describe("recovery dialog layout", () => {
  it("has a fixed height and a list that fills and scrolls inside it", () => {
    expect(rule(".eth-dialog.eth-recovery")).toMatch(/(^|;|\s)height:\s*min\(var\(--eth-size-recovery-dialog-height\)/);
    const list = rule(".eth-recovery__list");
    expect(list).toMatch(/flex:\s*1/);
    expect(list).toMatch(/max-height:\s*none/);
    expect(rule(".eth-recovery__intro")).toMatch(/min-height:/);
  });
});
