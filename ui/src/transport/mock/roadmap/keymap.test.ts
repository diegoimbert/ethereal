/** MockTransport: `Keymap::*` (v0.3, keymap), mirroring crates/ether-controller/tests/keymap.rs. */
import { describe, expect, it } from "vitest";
import type { Keymap } from "@/generated";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

const sample: Keymap = {
  preset: "AbletonLike",
  overrides: [
    { action: "edit.duplicate", chords: ["Mod+Shift+D", "Alt+D"] },
    { action: "transport.play", chords: [] },
  ],
};

describe("MockTransport keymap (keymap)", () => {
  const f = useMock();

  it("Get answers the default until something is stored", async () => {
    expect(await f.mock.send(cmd("Keymap", { type: "Get" }))).toEqual({ type: "Keymap", keymap: { preset: "Ethereal", overrides: [] } });
  });

  it("Set stores and emits Changed; Reset goes back to the default", async () => {
    await f.mock.send(cmd("Keymap", { type: "Set", keymap: sample }));
    expect(await f.mock.send(cmd("Keymap", { type: "Get" }))).toEqual({ type: "Keymap", keymap: sample });
    expect(f.events).toContainEqual({ type: "Keymap", event: { type: "Changed", keymap: sample } });
    f.events.length = 0;
    await f.mock.send(cmd("Keymap", { type: "Reset" }));
    expect(f.events).toContainEqual({ type: "Keymap", event: { type: "Changed", keymap: { preset: "Ethereal", overrides: [] } } });
    expect(await f.mock.send(cmd("Keymap", { type: "Get" }))).toEqual({ type: "Keymap", keymap: { preset: "Ethereal", overrides: [] } });
  });

  it("refuses invalid keymaps like the engine", async () => {
    await f.mock.send(cmd("Keymap", { type: "Set", keymap: sample }));
    const bad: Keymap["overrides"][] = [
      [{ action: "edit.copy", chords: ["Mod+c"] }],
      [{ action: "edit.copy", chords: ["Shift+Mod+C"] }],
      [{ action: "edit.copy", chords: ["Mod+C", "Mod+C"] }],
      [{ action: "edit.copy", chords: ["F1", "F2", "F3", "F4", "F5"] }],
      [
        { action: "b", chords: [] },
        { action: "a", chords: [] },
      ],
      [{ action: "", chords: [] }],
    ];
    for (const overrides of bad) {
      await expect(f.mock.send(cmd("Keymap", { type: "Set", keymap: { preset: "Ethereal", overrides } }))).rejects.toMatchObject({ code: "InvalidArgument" });
    }
    expect(await f.mock.send(cmd("Keymap", { type: "Get" }))).toEqual({ type: "Keymap", keymap: sample });
  });
});
