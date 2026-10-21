import { useMediaQuery } from "@/kit";

/**
 * The top bar is compact below this window width (it would clip its end otherwise): the
 * position shows the time alone (a click switches to bars), the computer keyboard's chip its
 * base note alone, and count-in, punch and the engine status move into a "more" menu. The app
 * shell marks the bar (`data-compact`) for the parts CSS alone hides.
 */
export const COMPACT_TOP_BAR_QUERY = "(max-width: 1439px)";

export const useCompactTopBar = (): boolean => useMediaQuery(COMPACT_TOP_BAR_QUERY);
