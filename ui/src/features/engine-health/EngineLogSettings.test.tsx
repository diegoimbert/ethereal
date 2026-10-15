import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EngineLogSettings } from "./EngineLogSettings";
import { engineLogIpc } from "./engineLog";

describe("EngineLogSettings", () => {
  const original = engineLogIpc.invoke;
  afterEach(() => {
    engineLogIpc.invoke = original;
  });

  it("opens the engine log, and shows why when it can't", async () => {
    const invoke = vi.fn().mockResolvedValueOnce(undefined).mockRejectedValueOnce("could not open a terminal: not found");
    engineLogIpc.invoke = invoke;
    render(<EngineLogSettings />);
    fireEvent.click(screen.getByRole("button", { name: "open engine log" }));
    expect(invoke).toHaveBeenCalledWith("open_engine_log");
    expect(screen.queryByRole("alert")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "open engine log" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("could not open a terminal: not found");
  });
});
