import { act, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { RenderingState } from "@/generated";
import { t } from "@/i18n";
import { useProjectStore } from "@/state";
import { useNotices } from "@/features/notifications";
import { refuseRecordWhileRendering, renderingText } from "./rendering";
import { renderWithMock, resetStores } from "./testUtils";
import { TransportBar } from "./index";

const store = () => useProjectStore.getState();

const render = (p: Partial<RenderingState>): RenderingState => ({ start: 0, end: 16, until: 4, progress: 0.25, preview: true, ...p });

/** The engine says a render runs (or, with `null`, that it ended). */
function setRendering(rendering: RenderingState | null) {
  act(() => {
    const { rendering: _, ...state } = store().transport!;
    store().setTransport(rendering ? { ...state, rendering } : state);
  });
}

afterEach(() => {
  resetStores();
  useNotices.getState().clear();
});

describe("render preview in the transport bar", () => {
  it("shows the progress; play and stop play the render, record waits", async () => {
    await renderWithMock(<TransportBar />);
    const play = screen.getByRole("button", { name: "play" });
    const stop = screen.getByTestId("transport-stop");
    const record = screen.getByRole("button", { name: "record" });

    setRendering(render({ progress: 0.42 }));
    const status = screen.getByTestId("render-preview");
    expect(status).toHaveTextContent("rendering 42%");
    expect(status).toHaveAttribute("title", t("shell.transport.rendering_title"));
    expect(screen.getByRole("progressbar", { name: "rendering 42%" })).toHaveAttribute("aria-valuenow", "42");
    expect(play).toBeEnabled();
    expect(play).toHaveAttribute("title", t("shell.transport.rendering_title"));
    expect(stop).toBeEnabled();
    expect(record).toBeDisabled();
    expect(record).toHaveAttribute("title", t("error.render.running"));

    // Nothing to preview: play can't start either.
    setRendering(render({ preview: false }));
    expect(play).toBeDisabled();
    expect(play).toHaveAttribute("title", t("error.render.no_preview"));
    expect(screen.getByTestId("render-preview")).toHaveAttribute("title", t("shell.transport.rendering_no_preview_title"));

    setRendering(null);
    expect(screen.queryByTestId("render-preview")).toBeNull();
    for (const b of [play, stop, record]) expect(b).toBeEnabled();
  });

  it("the palette's record entry says a render runs", async () => {
    await renderWithMock(<TransportBar />);
    expect(refuseRecordWhileRendering()).toBe(false);
    setRendering(render({}));
    expect(refuseRecordWhileRendering()).toBe(true);
    expect(useNotices.getState().notices.map((n) => n.message)).toContain(t("error.render.running"));
    expect(renderingText(render({ progress: 1.2 }))).toBe("rendering 100%");
  });
});
