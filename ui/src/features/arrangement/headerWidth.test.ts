import { afterEach, describe, expect, it } from "vitest";
import type { Track } from "@/generated";
import { HEADER_INDENT_PX, maxTrackDepth, MIN_HEADER_WIDTH, PHONE_HEADER_WIDTH } from "./layout";
import { resetArrangementUi, useArrangementUi } from "./uiStore";

const t = (id: string, parent: string | null = null) => ({ id, parent }) as unknown as Track;
const ui = () => useArrangementUi.getState();

afterEach(() => resetArrangementUi());

describe("the track header column", () => {
  it("measures the deepest group nesting", () => {
    expect(maxTrackDepth([t("a"), t("b")])).toBe(0);
    expect(maxTrackDepth([t("g1"), t("g2", "g1"), t("g3", "g2"), t("x", "g3"), t("y", "g1")])).toBe(3);
  });

  it("widens to fit the nesting, and goes back to the chosen width without it", () => {
    ui().setHeaderWidth(MIN_HEADER_WIDTH);
    ui().setHeaderDepth(5);
    expect(ui().headerWidth).toBe(MIN_HEADER_WIDTH + 5 * HEADER_INDENT_PX);
    ui().setHeaderDepth(0);
    expect(ui().headerWidth).toBe(MIN_HEADER_WIDTH);
  });

  it("keeps a chosen width wide enough already", () => {
    ui().setHeaderWidth(300);
    ui().setHeaderDepth(2);
    expect(ui().headerWidth).toBe(300);
  });

  it("can't be dragged narrower than the nesting needs", () => {
    ui().setHeaderDepth(4);
    ui().setHeaderWidth(100);
    expect(ui().headerWidth).toBe(MIN_HEADER_WIDTH + 4 * HEADER_INDENT_PX);
  });

  it("a phone's narrow column widens for nesting too", () => {
    ui().setPhoneHeader(true);
    expect(ui().headerWidth).toBe(PHONE_HEADER_WIDTH);
    ui().setHeaderDepth(2);
    expect(ui().headerWidth).toBe(PHONE_HEADER_WIDTH + 2 * HEADER_INDENT_PX);
  });
});
