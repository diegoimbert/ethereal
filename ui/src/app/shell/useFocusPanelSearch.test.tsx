import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useRef } from "react";
import { describe, expect, it } from "vitest";
import { useClickFocusesPanelSearch } from "./useFocusPanelSearch";

function Pane() {
  const ref = useRef<HTMLDivElement>(null);
  useClickFocusesPanelSearch(ref, "library");
  return (
    <div ref={ref}>
      <input type="search" aria-label="Search" />
      {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events -- a test fixture */}
      <div role="button" tabIndex={0} onClick={(e) => e.currentTarget.focus()}>
        Kick.wav
      </div>
      <input aria-label="Other field" />
      <span>empty space</span>
    </div>
  );
}

describe("useClickFocusesPanelSearch", () => {
  it("puts the focus back in the panel's search after a click inside it, but not from another field", async () => {
    render(<Pane />);
    const search = screen.getByRole("searchbox", { name: "Search" });
    fireEvent.click(screen.getByRole("button", { name: "Kick.wav" }));
    await waitFor(() => expect(document.activeElement).toBe(search));
    fireEvent.click(screen.getByText("empty space"));
    await waitFor(() => expect(document.activeElement).toBe(search));
    const other = screen.getByRole("textbox", { name: "Other field" });
    other.focus();
    fireEvent.click(other);
    await new Promise((r) => setTimeout(r, 10));
    expect(document.activeElement).toBe(other);
  });
});
