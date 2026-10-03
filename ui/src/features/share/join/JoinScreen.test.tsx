import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { InvitePreview, JoinFailure, JoinStage } from "@/generated";
import { JoinScreen } from "./JoinScreen";
import { FAILURE_TEXT, failureText } from "./texts";

const INVITE: InvitePreview = {
  host: { name: "Diego", color: 0x5cffe8 },
  project: "01J0000000000000000000000P",
  project_name: "Song",
  role: "Edit",
  online: [
    { name: "Diego", color: 0x5cffe8 },
    { name: "Tom", color: 0xff94a6 },
  ],
  local_copy: false,
};

function show(stage: JoinStage, extra: Partial<Parameters<typeof JoinScreen>[0]> = {}) {
  const onJoin = vi.fn();
  const onDismiss = vi.fn();
  const onOpenOfflineCopy = vi.fn();
  render(
    <JoinScreen
      open
      stage={stage}
      invite={INVITE}
      askName={false}
      onJoin={onJoin}
      onDismiss={onDismiss}
      onOpenOfflineCopy={onOpenOfflineCopy}
      {...extra}
    />,
  );
  return { onJoin, onDismiss, onOpenOfflineCopy };
}

describe("JoinScreen", () => {
  it.each(["Contacting", "Connecting"] as const)("%s: spinner and Cancel", (type) => {
    const { onDismiss } = show({ type });
    expect(screen.getByRole("status")).toHaveTextContent("Connecting to the invite…");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onDismiss).toHaveBeenCalled();
  });

  it("Ready: who invites you to what, your role, who's here; Join / Not now", () => {
    const { onJoin, onDismiss } = show({ type: "Ready", invite: INVITE });
    expect(screen.getByTestId("join-ready")).toHaveTextContent("Diego invites you to Song");
    expect(screen.getByText("You can edit")).toBeInTheDocument();
    expect(screen.getByLabelText("Also here: Tom")).toBeInTheDocument();
    expect(screen.queryByLabelText("Your name")).toBeNull();
    expect(screen.queryByText(/older copy/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    expect(onJoin).toHaveBeenCalledWith(null);
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(onDismiss).toHaveBeenCalled();
  });

  it("Ready: listen role and the local copy note", () => {
    show({ type: "Ready", invite: { ...INVITE, role: "Listen", local_copy: true, online: [INVITE.host] } });
    expect(screen.getByText("You can listen")).toBeInTheDocument();
    expect(screen.getByText(/You have an older copy of Song/)).toBeInTheDocument();
    expect(screen.queryByLabelText(/Also here/)).toBeNull();
  });

  it("Ready: asks for a name when there is no identity yet", () => {
    const { onJoin } = show({ type: "Ready", invite: INVITE }, { askName: true });
    const join = screen.getByRole("button", { name: "Join" });
    expect(join).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Your name"), { target: { value: "  Ada " } });
    expect(join).toBeEnabled();
    fireEvent.click(join);
    expect(onJoin).toHaveBeenCalledWith("Ada");
  });

  it("Ready: busy while the engine accepts", () => {
    show({ type: "Ready", invite: INVITE }, { busy: true });
    expect(screen.getByRole("button", { name: "Joining…" })).toBeDisabled();
  });

  it("Syncing: progress of the download", () => {
    show({ type: "Syncing", received_bytes: 250, total_bytes: 1000 });
    expect(screen.getByText("Downloading Song…")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "25");
  });

  it("Syncing: indeterminate without a total", () => {
    show({ type: "Syncing", received_bytes: 250, total_bytes: null });
    expect(screen.getByRole("progressbar")).not.toHaveAttribute("aria-valuenow");
  });

  it("HostOffline: waits, offers the offline copy", () => {
    const { onOpenOfflineCopy } = show({ type: "HostOffline", local_copy: "01J0000000000000000000000Q" });
    expect(screen.getByText(/Ethereal is closed\. Song opens as soon as they're back/)).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Waiting…");
    fireEvent.click(screen.getByRole("button", { name: "Open my offline copy" }));
    expect(onOpenOfflineCopy).toHaveBeenCalledWith("01J0000000000000000000000Q");
  });

  it("HostOffline without a copy: only Cancel", () => {
    show({ type: "HostOffline", local_copy: null });
    expect(screen.queryByRole("button", { name: "Open my offline copy" })).toBeNull();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it.each(Object.keys(FAILURE_TEXT) as JoinFailure[])("Failed %s: the message and Close", (reason) => {
    const { onDismiss } = show({ type: "Failed", reason, message: "" });
    expect(screen.getByRole("alert")).toHaveTextContent(FAILURE_TEXT[reason]);
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onDismiss).toHaveBeenCalled();
  });

  it("uses the engine's text for a damaged link only", () => {
    expect(failureText("BadLink", "This invite needs a newer version of Ethereal.")).toBe("This invite needs a newer version of Ethereal.");
    expect(failureText("InvalidInvite", "unknown room")).toBe(FAILURE_TEXT.InvalidInvite);
  });
});
