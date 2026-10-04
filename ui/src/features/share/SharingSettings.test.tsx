/** Settings > Advanced "Hide my IP (relay only)": every engine (native has its own TURN client, native-turn). */
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, ReplyValue } from "@/generated";
import { MockTransport, type SendOptions } from "@/transport";
import { SharingAdvancedSettings } from "./SharingSettings";
import { pushPreferences, useShareSettings } from "./settings";

class Recording extends MockTransport {
  sent: Command[] = [];
  override async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push(command);
    return super.send(command, opts);
  }
}
/** The desktop app's engine: a recording mock that reports the Tauri transport's kind. */
function desktop(): Recording {
  const t = new Recording({ timers: "manual" });
  Object.defineProperty(t, "kind", { value: "tauri" });
  return t;
}

const prefs = (t: { sent: Command[] }) => t.sent.filter((c) => c.domain === "Share" && c.command.type === "SetPreferences").map((c) => c.command);

afterEach(() => {
  localStorage.clear();
  useShareSettings.getState().reload();
});

describe("Hide my IP (relay only)", () => {
  it("works with the web engine", () => {
    const t = new Recording({ timers: "manual" });
    render(<SharingAdvancedSettings transport={t} />);
    const toggle = screen.getByRole("switch", { name: "Hide my IP (relay only)" });
    expect(toggle).toBeEnabled();
    expect(screen.getByText(/Connects through a TURN relay only/)).toBeInTheDocument();
    fireEvent.click(toggle);
    expect(useShareSettings.getState().relayOnly).toBe(true);
    expect(prefs(t).at(-1)).toMatchObject({ relay_only: true });
  });

  it("works on desktop too (the native endpoint has a TURN client)", () => {
    useShareSettings.getState().update({ relayOnly: true });
    const t = desktop();
    render(<SharingAdvancedSettings transport={t} />);
    const toggle = screen.getByRole("switch", { name: "Hide my IP (relay only)" });
    expect(toggle).toBeEnabled();
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(screen.queryByText(/not supported in the desktop app/)).not.toBeInTheDocument();
    pushPreferences(t);
    expect(prefs(t).at(-1)).toMatchObject({ relay_only: true });
    fireEvent.click(toggle);
    expect(useShareSettings.getState().relayOnly).toBe(false);
    expect(prefs(t).at(-1)).toMatchObject({ relay_only: false });
  });
});
