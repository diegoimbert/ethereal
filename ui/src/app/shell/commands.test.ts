import { act } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { registerDebugPanel, resetDebugPanels, useDebugPanels } from "@/features/debug";
import { buildCommands } from "./commands";

describe("palette commands", () => {
  // Here rather than in features/debug: this module pulls in most of the app, a cold import
  // that belongs to the file's import phase, not to a test's time budget.
  it("lists every registered debug panel", () => {
    const unregister = registerDebugPanel({ id: "pal", title: "Palette probe", render: () => "p" });
    try {
      const cmd = buildCommands(null, []).find((c) => c.label === "debug: Palette probe");
      expect(cmd).toBeDefined();
      act(() => cmd!.run());
      expect(useDebugPanels.getState().open).toContain("pal");
    } finally {
      act(() => {
        unregister();
        resetDebugPanels();
      });
    }
  });
});
