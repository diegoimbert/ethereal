// The web app's Content-Security-Policy. The source of truth is the deployed headers file
// (scripts/release/web/_headers, copied next to the site on Cloudflare Pages); `vite preview`
// serves the same policy so the production bundle is tested under it (ETHER_E2E_PREVIEW=1).
// `vite dev` doesn't: the React refresh preamble is an inline script.
import { appendFileSync, readFileSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";
import type { Plugin } from "vite";

export const HEADERS_FILE = fileURLToPath(new URL("../../scripts/release/web/_headers", import.meta.url));

/** The `Content-Security-Policy` value of the headers file (the app's, rule `/*`). */
export function readCsp(file = HEADERS_FILE): string {
  const csp = readPathCsps(file).get("/*");
  if (!csp) throw new Error(`no Content-Security-Policy in ${file}`);
  return csp;
}

/**
 * Each rule's `Content-Security-Policy`, by path: the app's (`/*`) and the pages that
 * replace it (`! Content-Security-Policy`, e.g. the WebCLAP ui frame, `/webclap-ui.html`).
 */
export function readPathCsps(file = HEADERS_FILE): Map<string, string> {
  const out = new Map<string, string>();
  let path: string | null = null;
  for (const line of readFileSync(file, "utf8").split("\n")) {
    if (/^\S/.test(line)) path = line.trim();
    const m = /^\s+Content-Security-Policy:\s*(.+)$/.exec(line);
    if (m?.[1] && path) out.set(path, m[1].trim());
  }
  return out;
}

/** Where the preview server's violation collector receives reports. */
const REPORT_PATH = "/__csp-report";

/**
 * Serves the policy on `vite preview`. With `ETHER_CSP_REPORTS=<file>` (the e2e run), browsers
 * also POST violation reports to the preview server, which appends them to that file (one JSON
 * per line) for the e2e teardown to check; `ETHER_CSP_REPORT_ONLY=1` serves the policy as
 * report-only (to find violations without breaking anything).
 */
export function cspPreview(): Plugin {
  return {
    name: "ether-csp-preview",
    configurePreviewServer(server) {
      const reports = process.env.ETHER_CSP_REPORTS;
      const reportOnly = process.env.ETHER_CSP_REPORT_ONLY === "1";
      const suffix = reports ? `; report-uri ${REPORT_PATH}` : "";
      const csps = readPathCsps();
      const csp = readCsp() + suffix;
      const header = reportOnly ? "Content-Security-Policy-Report-Only" : "Content-Security-Policy";
      server.middlewares.use((req, res, next) => {
        if (reports && req.url === REPORT_PATH && req.method === "POST") {
          let body = "";
          req.on("data", (c: Buffer) => (body += c.toString("utf8")));
          req.on("end", () => {
            appendFileSync(reports, body.replace(/\n/g, " ") + "\n");
            res.statusCode = 204;
            res.end();
          });
          return;
        }
        const own = csps.get((req.url ?? "").split("?")[0] ?? "");
        res.setHeader(header, own ? own + suffix : csp);
        next();
      });
    },
  };
}
