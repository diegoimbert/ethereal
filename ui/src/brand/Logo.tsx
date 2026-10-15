// The Ethereal logo: the faceted crystal of assets/brand/ethereal-logo.svg (the source of truth;
// Logo.test.ts keeps the two in sync), inline so it needs no asset URL (CSP, the desktop app).
// Brand artwork, not theme colors: it lives outside the token-checked folders (hardcoded.test.ts).
//
// Its box is the crystal's (550 × 840 of the 840 square). Sized by the `eth-logo` class: as tall
// as the text around it (1em) unless the caller's class sets a height.
import { useId } from "react";

type Gradient = { x1: number; y1: number; x2: number; y2: number; from: string; to: string };

/** The gradients, by index (a facet's fill `g<n>`). */
export const LOGO_GRADIENTS: Gradient[] = [
  { x1: 145, y1: 0, x2: 529.962, y2: 252.058, from: "#59B5F8", to: "#BDEEFF" },
  { x1: 420, y1: 0, x2: 804.962, y2: 252.058, from: "#B4A9FF", to: "#D6B5FA" },
  { x1: 420, y1: 225, x2: 681.729, y2: 506.154, from: "#E4F8FF", to: "#A8D6FF" },
  { x1: 330, y1: 545, x2: 494.673, y2: 595.239, from: "#BCEFFF", to: "#65B4FF" },
  { x1: 420, y1: 420, x2: 804.962, y2: 672.058, from: "#D8C4FF", to: "#9986F5" },
];

/** The facets, back to front: a path and a color or a gradient (`g<n>`). */
export const LOGO_FACETS: Array<[d: string, fill: string]> = [
  ["M420 0L145 420L295 322L420 0Z", "g0"],
  ["M420 0L295 322L420 225V0Z", "#DCF3FF"],
  ["M420 0V225L545 322L420 0Z", "#F0F0FF"],
  ["M420 0L545 322L695 420L420 0Z", "g1"],
  ["M145 420L295 322L420 225V545L145 420Z", "#B4D6FF"],
  ["M420 225L545 322L695 420L420 545V225Z", "g2"],
  ["M145 420L420 545L330 600L145 420Z", "#A3CEFF"],
  ["M145 420L330 600L420 840L145 420Z", "#C7E9FF"],
  ["M330 600L420 545V840L330 600Z", "g3"],
  ["M420 545L510 600L420 840V545Z", "#DAD8FF"],
  ["M510 600L695 420L420 840L510 600Z", "g4"],
  ["M420 545L695 420L510 600L420 545Z", "#B7A5F8"],
];

export interface LogoProps {
  className?: string;
  /** The accessible name; without it the logo is decorative (next to the "Ethereal" wordmark). */
  title?: string;
}

export function Logo({ className, title }: LogoProps) {
  // Gradient ids unique per instance: with two logos on a page, repeated ids would make every
  // copy paint with the first one's gradients (and lose them when it unmounts).
  const prefix = `eth-logo-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}-`;
  return (
    <svg
      className={className ? `eth-logo ${className}` : "eth-logo"}
      viewBox="145 0 550 840"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      focusable="false"
      data-testid="logo"
    >
      <defs>
        {LOGO_GRADIENTS.map((g, i) => (
          <linearGradient key={i} id={`${prefix}g${i}`} x1={g.x1} y1={g.y1} x2={g.x2} y2={g.y2} gradientUnits="userSpaceOnUse">
            <stop stopColor={g.from} />
            <stop offset="1" stopColor={g.to} />
          </linearGradient>
        ))}
      </defs>
      {LOGO_FACETS.map(([d, fill]) => (
        <path key={d} d={d} fill={fill.startsWith("#") ? fill : `url(#${prefix}${fill})`} />
      ))}
    </svg>
  );
}
