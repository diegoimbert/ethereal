import { describe, expect, it, vi } from "vitest";
import type { EngineTransport } from "@/transport";

describe("deleteStaleDrafts", () => {
  it("goes by the stored draft list, not the one this page loaded (#563: the tab that had the store kept a draft since)", async () => {
    // This page loads while another tab has an untouched draft "b"...
    localStorage.setItem("eth.project.drafts", JSON.stringify(["b"]));
    vi.resetModules();
    const { deleteStaleDrafts, isDraft } = await import("./drafts");
    expect(isDraft("b")).toBe(true);
    // ...which that tab then edits (kept: no longer a draft), and leaves "c" untouched.
    localStorage.setItem("eth.project.drafts", JSON.stringify(["c"]));
    const send = vi.fn(() => Promise.resolve({ type: "Unit" }));
    await deleteStaleDrafts({ send } as unknown as EngineTransport);
    const deleted = send.mock.calls.map((c) => (c as unknown as [{ command: { id: string } }])[0].command.id);
    expect(deleted).toEqual(["c"]);
    expect(isDraft("b")).toBe(false);
    expect(localStorage.getItem("eth.project.drafts")).toBeNull();
  });
});
