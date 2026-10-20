import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetStores } from "@/features/transport-bar/testUtils";
import { useMobileUi } from "@/features/mobile/mode";
import { useProjectStore } from "@/state";
import { resetShell } from "@/state/shell";
import { cmd, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { App } from "./App";
import { isChunkLoadError, saveAndReload } from "./crashRecovery";

/** Flip on to make the arrangement throw while rendering. */
const boom = vi.hoisted(() => ({ arrangement: false, mobile: false }));

vi.mock("@/features/arrangement", async (importOriginal) => {
  const real = await importOriginal<typeof import("@/features/arrangement")>();
  return {
    ...real,
    ArrangementView: () => {
      if (boom.arrangement) throw new Error("arrangement exploded");
      return <div data-testid="arrangement-ok" />;
    },
  };
});

vi.mock("@/features/mobile/MobileShell", async (importOriginal) => {
  if (boom.mobile) throw new TypeError("Failed to fetch dynamically imported module: /assets/MobileShell-abc.js");
  return importOriginal();
});

beforeEach(() => {
  resetShell();
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  // React logs caught render errors; the boundary logs its own line too.
  vi.spyOn(console, "error").mockImplementation(() => undefined);
});

afterEach(() => {
  cleanup();
  resetStores();
  boom.arrangement = false;
  boom.mobile = false;
  useMobileUi.setState({ enabled: false });
  vi.restoreAllMocks();
});

describe("error boundaries (#590)", () => {
  it("a crash in the arrangement shows its fallback; the transport bar and the rest keep working", async () => {
    boom.arrangement = true;
    render(<App />);
    const crash = screen.getByTestId("panel-crash");
    expect(crash.dataset.panel).toBe("arrangement");
    expect(within(crash).getByText("the arrangement crashed")).toBeTruthy();
    expect(within(crash).getByText("arrangement exploded")).toBeTruthy();
    // The rest of the editor is still there.
    expect(document.querySelector('[data-slot="transport-bar"]')!.childElementCount).toBeGreaterThan(0);
    expect(document.querySelector('[data-slot="project"]')!.childElementCount).toBeGreaterThan(0);

    // "reload panel" mounts it afresh.
    boom.arrangement = false;
    fireEvent.click(within(crash).getByRole("button", { name: "reload panel" }));
    expect(screen.getByTestId("arrangement-ok")).toBeTruthy();
    expect(screen.queryByTestId("panel-crash")).toBeNull();
  });

  it("a failed mobile chunk shows the app fallback with reload, not a blank window", async () => {
    boom.mobile = true;
    useMobileUi.setState({ enabled: true });
    render(<App />);
    const crash = await screen.findByTestId("panel-crash");
    expect(crash.dataset.panel).toBe("mobile editor");
    expect(within(crash).getByText("couldn't load the mobile editor")).toBeTruthy();
    expect(within(crash).getByRole("button", { name: "reload" })).toBeTruthy();
    expect(within(crash).queryByRole("button", { name: "reload panel" })).toBeNull();
  });

  it("recognizes chunk-load errors as browsers word them", () => {
    expect(isChunkLoadError(new TypeError("Failed to fetch dynamically imported module: https://x/a.js"))).toBe(true);
    expect(isChunkLoadError(new TypeError("Importing a module script failed."))).toBe(true);
    expect(isChunkLoadError(new Error("error loading dynamically imported module"))).toBe(true);
    expect(isChunkLoadError(new Error("x is undefined"))).toBe(false);
    expect(isChunkLoadError(null)).toBe(false);
  });

  it("reload saves the open project first (unsaved work isn't lost)", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 1 });
    render(
      <TransportProvider transport={mock}>
        <div />
      </TransportProvider>,
    );
    await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
    const sent = vi.spyOn(mock, "send");
    const reload = vi.fn();
    await act(() => saveAndReload(mock, reload));
    expect(sent).toHaveBeenCalledWith(cmd("Project", { type: "Save" }));
    expect(reload).toHaveBeenCalledTimes(1);
    expect(sent.mock.invocationCallOrder[0]!).toBeLessThan(reload.mock.invocationCallOrder[0]!);
    mock.dispose();
  });
});
