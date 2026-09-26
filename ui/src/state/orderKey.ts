/**
 * Fractional-index order keys (`OrderKey`), used to order siblings (tracks, devices,
 * scenes) without indices.
 *
 * This is a faithful port of the `fractional-indexing` npm package (David Greenspan's
 * algorithm, base-62 digits), which is also the scheme the Rust model uses
 * (`OrderKey::between`), so keys minted by the UI/mock and by the engine interleave
 * correctly. Keys compare with plain code-unit string comparison (`a < b`), never
 * `localeCompare`.
 *
 * Key layout: an "integer part" whose length is encoded by its head character
 * (`a`..`z` = 2..27 chars, `A`..`Z` = 27..2 chars, for negative integers), followed by an
 * optional fractional part that never ends in the zero digit.
 */

import type { OrderKey } from "@/generated";

export const BASE_62_DIGITS = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

const ZERO = BASE_62_DIGITS[0]!;
const LAST = BASE_62_DIGITS[BASE_62_DIGITS.length - 1]!;
/** The smallest integer part; it can't be decremented, only extended with fractions. */
const SMALLEST_INTEGER = "A" + ZERO.repeat(26);

function digit(d: string): number {
  const i = BASE_62_DIGITS.indexOf(d);
  if (i < 0) throw new Error(`invalid order key digit: ${d}`);
  return i;
}

/**
 * Midpoint of two fractional parts `a < b` (`b === null` = +infinity). Neither may end
 * in the zero digit.
 */
function midpoint(a: string, b: string | null): string {
  if (b !== null && a >= b) throw new Error(`${a} >= ${b}`);
  if (a.slice(-1) === ZERO || (b !== null && b.slice(-1) === ZERO)) throw new Error("trailing zero");
  if (b !== null) {
    // Skip the common prefix (treating a missing digit of `a` as zero).
    let n = 0;
    while ((a[n] ?? ZERO) === b[n]) n++;
    if (n > 0) return b.slice(0, n) + midpoint(a.slice(n), b.slice(n));
  }
  // First digits (or open ends) differ.
  const digitA = a ? digit(a[0]!) : 0;
  const digitB = b !== null ? digit(b[0]!) : BASE_62_DIGITS.length;
  if (digitB - digitA > 1) {
    return BASE_62_DIGITS[Math.round(0.5 * (digitA + digitB))]!;
  }
  // Consecutive digits.
  if (b !== null && b.length > 1) return b.slice(0, 1);
  // `b` is null or a single digit: take `a`'s first digit and recurse on the rest of `a`.
  return BASE_62_DIGITS[digitA]! + midpoint(a.slice(1), null);
}

function integerLength(head: string): number {
  if (head >= "a" && head <= "z") return head.charCodeAt(0) - "a".charCodeAt(0) + 2;
  if (head >= "A" && head <= "Z") return "Z".charCodeAt(0) - head.charCodeAt(0) + 2;
  throw new Error(`invalid order key head: ${head}`);
}

function validateInteger(int: string): void {
  if (int.length !== integerLength(int[0]!)) throw new Error(`invalid integer part of order key: ${int}`);
}

function integerPart(key: string): string {
  const len = integerLength(key[0] ?? "");
  if (len > key.length) throw new Error(`invalid order key: ${key}`);
  return key.slice(0, len);
}

/** Throws if `key` is not a well-formed order key. */
export function validateOrderKey(key: string): void {
  if (key === SMALLEST_INTEGER) throw new Error(`invalid order key: ${key}`);
  const int = integerPart(key);
  const frac = key.slice(int.length);
  if (frac.slice(-1) === ZERO) throw new Error(`invalid order key: ${key}`);
  for (const ch of key.slice(1)) digit(ch);
}

/** `true` if `key` is a well-formed order key. */
export function isValidOrderKey(key: string): boolean {
  try {
    validateOrderKey(key);
    return true;
  } catch {
    return false;
  }
}

function incrementInteger(x: string): string | null {
  validateInteger(x);
  const head = x[0]!;
  const digs = x.slice(1).split("");
  let carry = true;
  for (let i = digs.length - 1; carry && i >= 0; i--) {
    const d = digit(digs[i]!) + 1;
    if (d === BASE_62_DIGITS.length) {
      digs[i] = ZERO;
    } else {
      digs[i] = BASE_62_DIGITS[d]!;
      carry = false;
    }
  }
  if (!carry) return head + digs.join("");
  if (head === "Z") return "a" + ZERO;
  if (head === "z") return null;
  const h = String.fromCharCode(head.charCodeAt(0) + 1);
  if (h > "a") digs.push(ZERO);
  else digs.pop();
  return h + digs.join("");
}

function decrementInteger(x: string): string | null {
  validateInteger(x);
  const head = x[0]!;
  const digs = x.slice(1).split("");
  let borrow = true;
  for (let i = digs.length - 1; borrow && i >= 0; i--) {
    const d = digit(digs[i]!) - 1;
    if (d === -1) {
      digs[i] = LAST;
    } else {
      digs[i] = BASE_62_DIGITS[d]!;
      borrow = false;
    }
  }
  if (!borrow) return head + digs.join("");
  if (head === "a") return "Z" + LAST;
  if (head === "A") return null;
  const h = String.fromCharCode(head.charCodeAt(0) - 1);
  if (h < "Z") digs.push(LAST);
  else digs.pop();
  return h + digs.join("");
}

/**
 * A key strictly between `a` and `b` (`null` = open end). `keyBetween(null, null)` is
 * `"a0"`. Throws if `a >= b` or a key is malformed.
 */
export function keyBetween(a: OrderKey | null, b: OrderKey | null): OrderKey {
  if (a !== null) validateOrderKey(a);
  if (b !== null) validateOrderKey(b);
  if (a !== null && b !== null && a >= b) throw new Error(`${a} >= ${b}`);

  if (a === null) {
    if (b === null) return "a" + ZERO;
    const ib = integerPart(b);
    const fb = b.slice(ib.length);
    if (ib === SMALLEST_INTEGER) return ib + midpoint("", fb);
    if (ib < b) return ib;
    const res = decrementInteger(ib);
    if (res === null) throw new Error("cannot decrement any more");
    return res;
  }

  if (b === null) {
    const ia = integerPart(a);
    const fa = a.slice(ia.length);
    const i = incrementInteger(ia);
    return i === null ? ia + midpoint(fa, null) : i;
  }

  const ia = integerPart(a);
  const fa = a.slice(ia.length);
  const ib = integerPart(b);
  const fb = b.slice(ib.length);
  if (ia === ib) return ia + midpoint(fa, fb);
  const i = incrementInteger(ia);
  if (i === null) throw new Error("cannot increment any more");
  if (i < b) return i;
  return ia + midpoint(fa, null);
}

/**
 * `n` sorted keys strictly between `a` and `b` (evenly spread when both ends are set).
 */
export function keysBetween(a: OrderKey | null, b: OrderKey | null, n: number): OrderKey[] {
  if (n <= 0) return [];
  if (n === 1) return [keyBetween(a, b)];
  if (b === null) {
    let c = keyBetween(a, b);
    const out = [c];
    for (let i = 0; i < n - 1; i++) {
      c = keyBetween(c, b);
      out.push(c);
    }
    return out;
  }
  if (a === null) {
    let c = keyBetween(a, b);
    const out = [c];
    for (let i = 0; i < n - 1; i++) {
      c = keyBetween(a, c);
      out.push(c);
    }
    return out.reverse();
  }
  const mid = Math.floor(n / 2);
  const c = keyBetween(a, b);
  return [...keysBetween(a, c, mid), c, ...keysBetween(c, b, n - mid - 1)];
}

/** Comparator for sorting by order key (code-unit order, like Rust's byte order). */
export function compareOrderKeys(a: OrderKey, b: OrderKey): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/**
 * The key for inserting an item into `siblings` (already sorted by `order`) right before
 * the sibling with id `beforeId` (`null`, or an id that isn't a sibling = at the end).
 */
export function keyForInsert<T extends { id: string; order: OrderKey }>(
  siblings: ReadonlyArray<T>,
  beforeId: string | null,
): OrderKey {
  const idx = beforeId === null ? -1 : siblings.findIndex((s) => s.id === beforeId);
  if (idx < 0) return keyBetween(siblings.at(-1)?.order ?? null, null);
  return keyBetween(siblings[idx - 1]?.order ?? null, siblings[idx]!.order);
}
