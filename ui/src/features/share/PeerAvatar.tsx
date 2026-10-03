import clsx from "clsx";
import type { CSSProperties } from "react";
import type { Color } from "@/generated";
import { initials, peerColor } from "@/features/collab/store";

/** A person's avatar: initials on their colour (data, like track colours). */
export function PeerAvatar({
  name,
  color,
  size = "sm",
  offline,
  className,
}: {
  name: string;
  color: Color;
  size?: "sm" | "lg";
  offline?: boolean;
  className?: string;
}) {
  return (
    <span
      className={clsx("eth-share-avatar", `eth-share-avatar--${size}`, offline && "eth-share-avatar--offline", className)}
      style={{ "--eth-share-peer": peerColor(color) } as CSSProperties}
      aria-hidden
    >
      {initials(name)}
    </span>
  );
}

/** Up to `max` avatars, then "+N" (docs/SHARING.md §8.1). */
export function AvatarStack({ people, max = 3 }: { people: ReadonlyArray<{ name: string; color: Color }>; max?: number }) {
  if (people.length === 0) return null;
  const shown = people.slice(0, max);
  const more = people.length - shown.length;
  return (
    <span className="eth-share-stack" data-testid="share-avatars">
      {shown.map((p, i) => (
        <PeerAvatar key={`${i}-${p.name}`} name={p.name} color={p.color} />
      ))}
      {more > 0 && <span className="eth-share-stack__more">+{more}</span>}
    </span>
  );
}
