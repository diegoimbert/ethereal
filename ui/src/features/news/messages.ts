// news: the iframe's sandbox and the two messages an item page may post (NewsDialog.tsx).

/** The iframe's sandbox: scripts and popups, never same-origin: the page runs in an opaque origin (docs/NEWS.md). */
export const NEWS_SANDBOX = "allow-scripts allow-popups allow-popups-to-escape-sandbox";

type NewsMessage = { type: "ethereal-news"; action: "close" } | { type: "ethereal-news"; action: "resize"; height: number };

/** The page's message, if it is one of the two the app understands. */
export function parseNewsMessage(data: unknown): NewsMessage | null {
  if (!data || typeof data !== "object") return null;
  const d = data as Record<string, unknown>;
  if (d.type !== "ethereal-news") return null;
  if (d.action === "close") return { type: "ethereal-news", action: "close" };
  if (d.action === "resize" && typeof d.height === "number" && Number.isFinite(d.height)) return { type: "ethereal-news", action: "resize", height: d.height };
  return null;
}
