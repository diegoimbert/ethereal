import type { CSSProperties } from "react";
import type { ProjectSummary } from "@/generated";
import { initials, peerColor } from "@/features/collab/store";
import { Badge } from "@/kit";
import { avatarStack, isLive, shareBadge, useShareSession } from "./shareState";

/**
 * A shared project's marks in the project screen (docs/SHARING.md §8.5): "Shared" (this app
 * hosts it), "From <host>" (an offline copy) or "Sharing ended"; the last known
 * participants (at most 3 avatars, then "+N"); "Live" while it is the open, connected
 * project. Nothing for a private project.
 */
export function ShareBadges({ project }: { project: ProjectSummary }) {
  const state = useShareSession((s) => s.state);
  const badge = shareBadge(project);
  if (!badge || !project.share) return null;
  const live = project.share.active && isLive(state, project.id);
  const { shown, more } = avatarStack(project.share.participants);
  const everyone = project.share.participants.map((p) => p.name).join(", ");
  return (
    <span className="eth-project-share" data-testid={`share-${project.id}`}>
      <span title={badge.title}>
        <Badge tone={badge.tone}>{badge.label}</Badge>
      </span>
      {shown.length > 0 && (
        <span className="eth-project-share__avatars" role="img" aria-label={`People: ${everyone}`} title={everyone}>
          {shown.map((p, i) => (
            <span key={i} className="eth-project-share__avatar" style={{ "--eth-project-peer": peerColor(p.color) } as CSSProperties} aria-hidden>
              {initials(p.name)}
            </span>
          ))}
          {more > 0 && (
            <span className="eth-project-share__more" aria-hidden>
              +{more}
            </span>
          )}
        </span>
      )}
      {live && (
        <span className="eth-project-share__live" title="Connected now">
          <span className="eth-project-share__dot" aria-hidden />
          Live
        </span>
      )}
    </span>
  );
}
