// The deployed Content-Security-Policy (scripts/release/web/_headers) allows index.html's
// inline script by its hash: editing that script (even its whitespace) must update the hash.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";
import { describe, expect, it } from "vitest";
import { readCsp } from "../csp.ts";

const directive = (csp: string, name: string): string[] =>
  csp
    .split(";")
    .map((d) => d.trim().split(/\s+/))
    .find(([n]) => n === name)
    ?.slice(1) ?? [];

describe("Content-Security-Policy", () => {
  const csp = readCsp();
  const html = readFileSync(fileURLToPath(new URL("../index.html", import.meta.url)), "utf8");

  it("allows each inline script of index.html by its hash", () => {
    const inline = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1] ?? "");
    expect(inline.length).toBeGreaterThan(0);
    const scriptSrc = directive(csp, "script-src");
    for (const code of inline) {
      const hash = `'sha256-${createHash("sha256").update(code).digest("base64")}'`;
      expect(scriptSrc, "update script-src in scripts/release/web/_headers").toContain(hash);
    }
  });

  it("keeps scripts strict", () => {
    const scriptSrc = directive(csp, "script-src");
    expect(scriptSrc).not.toContain("'unsafe-inline'");
    expect(scriptSrc).not.toContain("'unsafe-eval'");
    expect(directive(csp, "object-src")).toEqual(["'none'"]);
    expect(directive(csp, "frame-ancestors")).toEqual(["'none'"]);
  });
});
