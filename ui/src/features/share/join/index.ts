// `join-flow` (docs/SHARING.md §5, §8.3): deep links, the join screen, "Join with a link…".
import { openJoinWithLink } from "./store";

export { JoinRoot } from "./JoinRoot";
export { JoinScreen, type JoinScreenProps } from "./JoinScreen";
export { FAILURE_TEXT, failureText } from "./texts";
export { JoinWithLinkDialog, JoinWithLinkField, type JoinWithLinkFieldProps } from "./JoinWithLink";
export { IDENTITY_KEY, loadIdentity, saveIdentity, type ShareIdentity } from "./identity";
export { JoinLanding, type JoinLandingProps } from "./JoinLanding";
export { isMobile, joinRoute, prefersBrowser, setPrefersBrowser, type JoinRoute } from "./landing";
export { deepLinkForTail, inviteLinkFrom, joinTail, linkProblem } from "./links";
export { openInvite, openJoinWithLink } from "./store";

/**
 * Command palette entries (`ui/src/app/shell/commands.ts` spreads them in): "Join shared
 * project…" opens the "Join with a link…" dialog.
 */
export function joinPaletteCommands(): { id: string; label: string; group: string; keywords: string; run(): void }[] {
  return [
    {
      id: "share:join-link",
      group: "Share",
      label: "Join shared project…",
      keywords: "invite link paste collaborate share session join",
      run: () => openJoinWithLink(),
    },
  ];
}
