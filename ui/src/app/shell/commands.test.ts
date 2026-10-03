import { afterEach, describe, expect, it } from "vitest";
import { useJoinStore } from "@/features/share/join/store";
import { buildCommands } from "./commands";

afterEach(() => useJoinStore.setState({ pasteOpen: false }));

describe("palette commands", () => {
  it("offers 'Join shared project…' (join-flow), with or without an engine", () => {
    const join = buildCommands(null, []).find((c) => c.id === "share:join-link");
    expect(join).toMatchObject({ label: "Join shared project…", group: "Share" });
    join!.run();
    expect(useJoinStore.getState().pasteOpen).toBe(true);
  });
});
