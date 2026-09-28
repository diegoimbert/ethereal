import { describe, expect, it } from "vitest";
import type { TrackInput } from "@/generated";
import { TAP_ARM_UNSUPPORTED, tapArmBlocked } from "./tapRecording";

const tap: TrackInput = { type: "Track", track: "01JXT21Q00TCH1Q3BRJMTQCWHX", tap: "PostFader" };

describe("tap recording arm guard", () => {
  it("blocks arming a tapped track in the browser build", () => {
    expect(tapArmBlocked("wasm", tap, false)).toBe(TAP_ARM_UNSUPPORTED);
  });

  it("always allows disarming", () => {
    expect(tapArmBlocked("wasm", tap, true)).toBeNull();
  });

  it("allows hardware inputs in the browser and taps on native hosts", () => {
    expect(tapArmBlocked("wasm", { type: "Audio", first: 0, count: 2 }, false)).toBeNull();
    expect(tapArmBlocked("wasm", { type: "None" }, false)).toBeNull();
    for (const host of ["tauri", "remote", "mock", undefined] as const) {
      expect(tapArmBlocked(host, tap, false)).toBeNull();
    }
  });
});
