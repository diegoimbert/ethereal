// Preview runs (`ETHER_E2E_PREVIEW=1`): the preview server serves the deployed
// Content-Security-Policy and appends the browsers' violation reports to
// `ETHER_CSP_REPORTS` (apps/web/csp.ts). Any report fails the run: the policy would break
// (or, report-only, would break) something the specs exercised.
import { existsSync, readFileSync } from "node:fs";

export default function globalTeardown(): void {
  const file = process.env.ETHER_CSP_REPORTS;
  if (!file || !existsSync(file)) return;
  const lines = readFileSync(file, "utf8").split("\n").filter(Boolean);
  if (lines.length === 0) return;
  const seen = new Set<string>();
  for (const line of lines) {
    let r: Record<string, unknown> = {};
    try {
      r = (JSON.parse(line) as { "csp-report"?: Record<string, unknown> })["csp-report"] ?? {};
    } catch {
      /* printed raw below */
    }
    const key = `${String(r["effective-directive"] ?? r["violated-directive"] ?? "?")} ${String(r["blocked-uri"] ?? line)}`;
    if (seen.has(key)) continue;
    seen.add(key);
    console.error(`CSP violation: ${key} (on ${String(r["document-uri"] ?? "?")})`);
  }
  throw new Error(`${lines.length} Content-Security-Policy violation report(s), see above (${file})`);
}
