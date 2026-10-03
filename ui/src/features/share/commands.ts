// Command palette entries for sharing (docs/SHARING.md §8). The palette's "Join shared
// project…" belongs to join-flow.
import { cmd, type EngineTransport } from "@/transport";
import { openSettings } from "@/features/audio-settings/store";
import { useCollabStore } from "@/features/collab/store";
import { attempt } from "./actions";
import { useConfirm } from "./confirmStore";
import { useShareStore } from "./store";
import { shareToast } from "./toasts";

/** Same shape as the palette's `PaletteCommand` (app/shell/commands.ts). */
export interface ShareCommandEntry {
  id: string;
  label: string;
  group: string;
  keywords?: string;
  run(): void;
}

export function shareCommands(transport: EngineTransport | null, hasProject: boolean): ShareCommandEntry[] {
  const out: ShareCommandEntry[] = [];
  const state = useShareStore.getState().state;
  const openPopover = () => useShareStore.getState().setPopoverOpen(true);
  if (transport && hasProject) {
    if (state.type === "Off" && useCollabStore.getState().status.type === "Offline") {
      out.push({
        id: "share:start",
        group: "Share",
        label: "Share project…",
        keywords: "share invite link collaborate together peer",
        run: () => void attempt(transport, cmd("Share", { type: "Start" }), "Couldn't share this project").then((ok) => ok && openPopover()),
      });
    }
    if (state.type === "Hosting") {
      const link = state.edit_link;
      out.push({ id: "share:open", group: "Share", label: "Share: Show people and links", keywords: "share invite popover participants", run: openPopover });
      if (link) {
        out.push({
          id: "share:copy",
          group: "Share",
          label: "Share: Copy edit link",
          keywords: "share invite link copy clipboard",
          run: () =>
            void navigator.clipboard.writeText(link).then(
              () => shareToast("Link copied", "Anyone with this link can edit."),
              () => openPopover(),
            ),
        });
      }
      out.push({
        id: "share:stop",
        group: "Share",
        label: "Stop sharing…",
        keywords: "share stop end revoke links",
        run: () =>
          useConfirm.getState().ask({
            title: "Stop sharing?",
            body: "Everyone keeps an offline copy. Links stop working.",
            action: "Stop sharing",
            danger: true,
            run: () => void attempt(transport, cmd("Share", { type: "Stop" }), "Couldn't stop sharing"),
          }),
      });
    }
    if (state.type === "Joined") {
      out.push({
        id: "share:leave",
        group: "Share",
        label: "Leave shared project",
        keywords: "share leave quit disconnect session",
        run: () => void attempt(transport, cmd("Share", { type: "Leave" }), "Couldn't leave"),
      });
    }
  }
  out.push({
    id: "settings:sharing",
    group: "Appearance",
    label: "Sharing settings…",
    keywords: "share name colour color identity signaling ice turn stun relay engine server remote advanced preferences",
    run: () => openSettings("sharing"),
  });
  return out;
}
