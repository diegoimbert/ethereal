/**
 * Whether a control's hover readout (`readout`, shown above it) would be cut by the top of the
 * tracks' scroll area (the first visible row): then it shows below the control instead.
 * Controls outside the tracks (a rack chain) always show it above.
 */
export function readoutBelow(control: Element, readout: Element | null): boolean {
  const scroll = control.closest(".eth-arr__scroll");
  if (!scroll || !readout) return false;
  const room = control.getBoundingClientRect().top - scroll.getBoundingClientRect().top;
  return room < readout.getBoundingClientRect().height;
}
