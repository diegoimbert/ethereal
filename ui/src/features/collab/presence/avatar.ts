import type { CSSProperties } from "react";
import type { Color } from "@/generated";
import { inkOn } from "@/features/arrangement/helpers";
import { peerColor } from "../store";

/**
 * Inline custom properties for a peer avatar: its colour as the fill and whichever ink
 * (`--eth-ink-dark` / `--eth-ink-light`) reads best on it, in either theme. Peer colours
 * are data, like track colours, so the ink follows the colour, not the theme.
 */
export function avatarStyle(color: Color): CSSProperties {
  return { "--eth-collab-peer": peerColor(color), "--eth-collab-peer-ink": inkOn(color) } as CSSProperties;
}
