import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { OtherTabScreen } from "./OtherTabScreen";

describe("OtherTabScreen (#563)", () => {
  it("says another tab has ethereal, and use here waits for it", async () => {
    let done!: () => void;
    const onUseHere = vi.fn(() => new Promise<void>((r) => (done = r)));
    render(<OtherTabScreen reason="busy" onUseHere={onUseHere} />);
    expect(screen.getByRole("heading")).toHaveTextContent("ethereal is open in another tab");
    fireEvent.click(screen.getByRole("button", { name: "use here" }));
    expect(onUseHere).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "waiting for the other tab…" })).toBeDisabled();
    done();
  });

  it("a failed take-over says why and can be tried again", async () => {
    render(<OtherTabScreen reason="moved" onUseHere={() => Promise.reject(new Error("no lock"))} />);
    expect(screen.getByRole("heading")).toHaveTextContent("ethereal moved to another tab");
    fireEvent.click(screen.getByRole("button", { name: "use here" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("no lock");
    expect(screen.getByRole("button", { name: "use here" })).toBeEnabled();
  });
});
