/**
 * Whether a control's hover readout (`readout`, shown above it) would be cut by the top of the
 * scroll area around it (the tracks' first visible row, the master track's own area): then it
 * shows below the control instead. Controls outside the arrangement (a rack chain) always show
 * it above.
 */
export function readoutBelow(control: Element, readout: Element | null): boolean {
  const scroll = control.closest(".eth-arr__scroll, .eth-arr__master");
  if (!scroll || !readout) return false;
  const room = control.getBoundingClientRect().top - scroll.getBoundingClientRect().top;
  return room < readout.getBoundingClientRect().height;
}
