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

  it("a control without a title shows its label, unless the label only repeats its text", () => {
    render(
      <>
        <button type="button" aria-label="mute Kick">
          <svg />
        </button>
        <button type="button" aria-label="add track">
          add track
        </button>
        <div role="slider" aria-label="Kick pan" aria-valuenow={0.5} tabIndex={0} />
        <TitleTooltipHost />
      </>,
    );
    act(() => void fireEvent.pointerOver(screen.getByRole("button", { name: "mute Kick" })));
    expect(screen.getByRole("tooltip")).toHaveTextContent("mute Kick");
    act(() => void fireEvent.pointerOver(screen.getByRole("button", { name: "add track" })));
    expect(screen.queryByRole("tooltip")).toBeNull();
    act(() => void fireEvent.pointerOver(screen.getByRole("slider", { name: "Kick pan" })));
    expect(screen.getByRole("tooltip")).toHaveTextContent("Kick pan");
    // A slider's tooltip goes below it (its value shows above on hover).
    expect(screen.getByRole("tooltip")).toHaveClass("eth-tooltip--bottom");
  });
});
