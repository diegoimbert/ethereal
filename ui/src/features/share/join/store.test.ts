import { beforeEach, describe, expect, it } from "vitest";
import { useProjectScreen } from "@/features/project/screenStore";
import { openInvite, useJoinStore } from "./store";

describe("openInvite", () => {
  beforeEach(() => {
    useJoinStore.setState({ queue: [] });
  });

  it("cancels the launch project screen, so the joined project doesn't open under it", () => {
    useProjectScreen.setState({ launchPending: true, open: false });
    openInvite("https://etherealws.pages.dev/join/abc#key");
    expect(useProjectScreen.getState().launchPending).toBe(false);
    expect(useProjectScreen.getState().open).toBe(false);
    expect(useJoinStore.getState().queue.map((q) => q.link)).toEqual(["https://etherealws.pages.dev/join/abc#key"]);
  });

  it("closes a project screen that is already showing (a deep link at launch)", () => {
    useProjectScreen.setState({ launchPending: false, open: true, mode: "new" });
    openInvite("ethereal://join/abc#key");
    expect(useProjectScreen.getState().open).toBe(false);
    expect(useProjectScreen.getState().mode).toBe("home");
  });
});
