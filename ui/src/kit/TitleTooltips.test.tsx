import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { TitleTooltipHost } from "./TitleTooltips";

describe("TitleTooltipHost", () => {
  it("shows a hovered element's title as the kit tooltip, holding the title aside meanwhile", () => {
    render(
      <>
        <button type="button" title="Punch in/out: record only inside the loop region">
          PUNCH
        </button>
        <span>elsewhere</span>
        <TitleTooltipHost />
      </>,
    );
    const button = screen.getByRole("button", { name: "PUNCH" });
    act(() => void fireEvent.pointerOver(button));
    expect(screen.getByRole("tooltip")).toHaveTextContent("Punch in/out: record only inside the loop region");
    // No native tooltip on top of it.
    expect(button).not.toHaveAttribute("title");

    // Moving away hides it and puts the title back.
    act(() => void fireEvent.pointerOver(screen.getByText("elsewhere")));
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect(button).toHaveAttribute("title", "Punch in/out: record only inside the loop region");

    // A press hides it too.
    act(() => void fireEvent.pointerOver(button));
    act(() => void fireEvent.pointerDown(button));
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect(button).toHaveAttribute("title");
  });
});
