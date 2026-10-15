// The web app's Content-Security-Policy. The source of truth is the deployed headers file
// (scripts/release/web/_headers, copied next to the site on Cloudflare Pages); `vite preview`
// serves the same policy so the production bundle is tested under it (ETHER_E2E_PREVIEW=1).
// `vite dev` doesn't: the React refresh preamble is an inline script.
import { appendFileSync, readFileSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";
import type { Plugin } from "vite";

export const HEADERS_FILE = fileURLToPath(new URL("../../scripts/release/web/_headers", import.meta.url));

/** The `Content-Security-Policy` value of the headers file. */
export function readCsp(file = HEADERS_FILE): string {
  const m = /^\s*Content-Security-Policy:\s*(.+)$/m.exec(readFileSync(file, "utf8"));
  if (!m?.[1]) throw new Error(`no Content-Security-Policy in ${file}`);
  return m[1].trim();
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
      const csp = readCsp() + (reports ? `; report-uri ${REPORT_PATH}` : "");
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
        res.setHeader(header, csp);
        next();
      });
    },
  };
}
