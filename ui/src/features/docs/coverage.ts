// The docs' coverage of the explain inventory (docs-site/inventory.json): every item that
// still applies has a page and a heading that explain it (`covers:` in the page's
// front-matter). Pure; `coverage.test.ts` runs it on the real files and keeps
// docs-site/coverage.json in sync.
import type { Docs } from "./model";

export interface InventoryItem {
  id: string;
  title: string;
  /** Set when the item no longer applies: why. */
  removed?: string;
}

export interface Inventory {
  areas: Array<{ id: string; name: string; items: InventoryItem[] }>;
}

export interface CoverageReport {
  total: number;
  removed: number;
  covered: number;
  pending: number;
  /** Covered share of the items that still apply, in percent (one decimal). */
  percent: number;
  /** Item id → `<page path>#<anchor>` (all pages covering it). */
  items: Record<string, string[]>;
  /** Item id → why it was removed. */
  removedItems: Record<string, string>;
  /** Items not written yet (docs-site/pending.txt). */
  pendingItems: string[];
}

export interface CoverageProblems {
  /** `covers:` lines naming an item the inventory doesn't have (or has removed). */
  unknownItems: string[];
  /** `covers:` lines whose anchor isn't a heading on the page. */
  missingAnchors: string[];
  /** Items that still apply, with no page and not pending. */
  uncovered: string[];
  /** Pending lines that are covered now, or unknown. */
  stalePending: string[];
  /** Removed items without a reason. */
  removedWithoutReason: string[];
}

/**
 * Reads docs-site/pending.txt: one inventory item id per line, and `page <section>/<page>` for
 * pages that are linked to but not written yet; `#` starts a comment.
 */
export function parsePending(text: string): { items: string[]; pages: string[] } {
  const items: string[] = [];
  const pages: string[] = [];
  for (const raw of text.split("\n")) {
    const line = raw.replace(/#.*/, "").trim();
    if (!line) continue;
    const page = /^page\s+(\S+)$/.exec(line);
    if (page) pages.push(page[1]!);
    else items.push(line);
  }
  return { items, pages };
}

export function checkCoverage(docs: Docs, inventory: Inventory, pending: ReadonlyArray<string>): { report: CoverageReport; problems: CoverageProblems } {
  const all = inventory.areas.flatMap((a) => a.items);
  const byId = new Map(all.map((i) => [i.id, i]));
  const items: Record<string, string[]> = {};
  const problems: CoverageProblems = { unknownItems: [], missingAnchors: [], uncovered: [], stalePending: [], removedWithoutReason: [] };
  for (const page of docs.pages) {
    const anchors = new Set(page.headings.map((h) => h.id));
    for (const c of page.covers) {
      const item = byId.get(c.item);
      if (!item || item.removed !== undefined) problems.unknownItems.push(`${page.path}: ${c.item}${item ? " (removed)" : ""}`);
      if (!anchors.has(c.anchor)) problems.missingAnchors.push(`${page.path}: #${c.anchor} (for ${c.item})`);
      (items[c.item] ??= []).push(`${page.path}#${c.anchor}`);
    }
  }
  const pendingSet = new Set(pending);
  const removedItems: Record<string, string> = {};
  for (const item of all) {
    if (item.removed !== undefined) {
      if (!item.removed.trim()) problems.removedWithoutReason.push(item.id);
      removedItems[item.id] = item.removed;
      continue;
    }
    if (!items[item.id] && !pendingSet.has(item.id)) problems.uncovered.push(item.id);
  }
  for (const id of pending) {
    const item = byId.get(id);
    if (!item || item.removed !== undefined || items[id]) problems.stalePending.push(id);
  }
  const removed = Object.keys(removedItems).length;
  const live = all.length - removed;
  const covered = all.filter((i) => i.removed === undefined && items[i.id]).length;
  const sorted: Record<string, string[]> = {};
  for (const item of all) if (items[item.id]) sorted[item.id] = items[item.id]!;
  return {
    report: {
      total: all.length,
      removed,
      covered,
      pending: pending.length,
      percent: live === 0 ? 100 : Math.round((covered / live) * 1000) / 10,
      items: sorted,
      removedItems,
      pendingItems: [...pending],
    },
    problems,
  };
}
