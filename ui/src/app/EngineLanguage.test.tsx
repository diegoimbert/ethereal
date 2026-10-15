import { act, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { resetI18nForTests, setLocale } from "@/i18n/runtime";
import { defaultTrackName, isDefaultChainName, tracksOrdered, useProjectStore } from "@/state";
import { EngineLanguage } from "./EngineLanguage";

afterEach(() => {
  resetI18nForTests();
  resetStores();
});

describe("default names follow the UI's language", () => {
  it("tells the engine the language at start and on a switch", async () => {
    const { mock } = await renderWithMock(<EngineLanguage />);
    const send = vi.spyOn(mock, "send");
    await act(() => setLocale("fr", { persist: false }));
    await waitFor(() => expect(send).toHaveBeenCalledWith(expect.objectContaining({ domain: "Settings", command: { type: "SetLanguage", language: "fr" } })));
  });

  it("names tracks and chains like the engine, in the UI's language", async () => {
    await renderWithMock(<EngineLanguage />);
    const project = useProjectStore.getState().project!;
    const group = tracksOrdered(project).find((t) => t.kind === "Group");
    await setLocale("fr", { persist: false });
    if (group) expect(defaultTrackName(project, group)).toMatch(/^\d+ groupe$/);
    expect(defaultTrackName(project, { id: "x", kind: "Return" })).toMatch(/^retour [a-z]$/);
    // A chain's default name, in French or in English (a project made before the switch).
    expect(isDefaultChainName("chaîne 2")).toBe(true);
    expect(isDefaultChainName("chain 12")).toBe(true);
    expect(isDefaultChainName("lead chain")).toBe(false);
  });
});
