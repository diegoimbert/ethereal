/**
 * The device library's arrow keys: one row down or up, wrapping around the ends (Up on the
 * first row goes to the last, Down on the last to the first). `from` is -1 when no row is
 * active: Down then starts at the first row, Up at the last. Returns -1 for an empty list.
 */
export function stepWrapped(from: number, length: number, dir: 1 | -1): number {
  if (length <= 0) return -1;
  if (from < 0 || from >= length) return dir === 1 ? 0 : length - 1;
  return (from + dir + length) % length;
}
