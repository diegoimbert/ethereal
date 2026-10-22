import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { LoadProgress } from "@/generated";
import { t } from "@/i18n";
import { useProjectStore } from "@/state";
import { useNotices } from "@/features/notifications";
import { loadFraction, loadingText, refuseWhileLoading } from "./loading";
import { renderWithMock, resetStores } from "./testUtils";
import { TransportBar } from "./index";

const store = () => useProjectStore.getState();

const progress = (p: Partial<LoadProgress>): LoadProgress => ({ plugins_loaded: 0, plugins_total: 0, media_loaded: 0, media_total: 0, ...p });

/** The engine says the open project loads (or, with `null`, that it is ready). */
function setLoading(loading: LoadProgress | null) {
  act(() => {
    const { loading: _, ...state } = store().transport!;
    store().setTransport(loading ? { ...state, loading } : state);
  });
}

afterEach(() => {
  resetStores();
  useNotices.getState().clear();
});

describe("project loading", () => {
  it("says what loads: plugins, then audio files, then the audio starting", () => {
    expect(loadingText(progress({ plugins_loaded: 2, plugins_total: 5, media_total: 3 }))).toBe("loading plugins 2/5");
    expect(loadingText(progress({ plugins_loaded: 5, plugins_total: 5, media_loaded: 1, media_total: 3 }))).toBe("loading audio files 1/3");
    expect(loadingText(progress({ plugins_loaded: 5, plugins_total: 5 }))).toBe("starting the audio…");
    expect(loadFraction(progress({ plugins_loaded: 2, plugins_total: 5, media_loaded: 1, media_total: 5 }))).toBeCloseTo(0.3);
    expect(loadFraction(progress({}))).toBe(1);
  });

  it("disables play, stop, record and the space bar while loading, and shows the progress", async () => {
    await renderWithMock(<TransportBar />);
    const play = screen.getByRole("button", { name: "play" });
    const stop = screen.getByTestId("transport-stop");
    const record = screen.getByRole("button", { name: "record" });
    expect(play).toBeEnabled();

    setLoading(progress({ plugins_loaded: 2, plugins_total: 5 }));
    for (const b of [play, stop, record]) expect(b).toBeDisabled();
    expect(play).toHaveAttribute("title", t("error.project.loading"));
    const status = screen.getByTestId("project-loading");
    expect(status).toHaveTextContent("loading plugins 2/5");
    expect(screen.getByRole("progressbar", { name: "loading plugins 2/5" })).toHaveAttribute("aria-valuenow", "40");
    fireEvent.keyDown(window, { key: " " });
    await act(() => Promise.resolve());
    expect(store().transport?.playing).toBe(false);
    // The rest of the bar works: the project can be explored and edited meanwhile.
    expect(screen.getByRole("button", { name: "loop" })).toBeEnabled();
    expect(screen.getByLabelText("tempo")).toBeEnabled();

    setLoading(progress({ plugins_loaded: 5, plugins_total: 5, media_loaded: 1, media_total: 2 }));
    expect(screen.getByTestId("project-loading")).toHaveTextContent("loading audio files 1/2");

    // Ready.
    setLoading(null);
    expect(screen.queryByTestId("project-loading")).toBeNull();
    for (const b of [play, stop, record]) expect(b).toBeEnabled();
    fireEvent.keyDown(window, { key: " " });
    await waitFor(() => expect(store().transport?.playing).toBe(true));
  });

  it("the palette's transport entries say the project is loading instead of playing", async () => {
    await renderWithMock(<TransportBar />);
    expect(refuseWhileLoading()).toBe(false);
    setLoading(progress({ media_total: 1 }));
    expect(refuseWhileLoading()).toBe(true);
    expect(useNotices.getState().notices.map((n) => n.message)).toContain(t("error.project.loading"));
  });
});
