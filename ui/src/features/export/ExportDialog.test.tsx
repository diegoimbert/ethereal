import { useEffect } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { pickOption } from "@/kit/testing";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, ExportRequest, ReplyValue } from "@/generated";
import { useProjectStore } from "@/state";
import { TransportProvider, type SendOptions } from "@/transport";
import { createEmptyProject, MockTransport } from "@/transport/mock";
import { useTransportSwitch } from "@/transport/createDefaultTransport";
import { ExportDialog, useExportAudio } from "./index";
import { buildRequest, DEFAULT_FORM, defaultStemTracks, estimateBytes, formProblem, LARGE_EXPORT_BYTES } from "./request";

class SpyMock extends MockTransport {
  sent: Command[] = [];
  override send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push(command);
    return super.send(command, opts);
  }
  renders(): ExportRequest[] {
    return this.sent.flatMap((c) => (c.domain === "Export" && c.command.type === "Render" ? [c.command.request] : []));
  }
}

async function setup() {
  const mock = new SpyMock({ timers: "manual", seed: 5 });
  render(
    <TransportProvider transport={mock}>
      <ExportDialog />
    </TransportProvider>,
  );
  await waitFor(() => {
    if (!useProjectStore.getState().project) throw new Error("not connected");
  });
  // Opened from the project menu.
  act(() => useExportAudio.getState().show());
  const dialog = await screen.findByRole("dialog", { name: "export audio" });
  return { mock, dialog };
}

describe("ExportDialog", () => {
  const created: string[] = [];
  beforeEach(() => {
    created.length = 0;
    URL.createObjectURL = vi.fn(() => {
      created.push("blob");
      return "blob:mock";
    });
    URL.revokeObjectURL = vi.fn();
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => undefined);
  });
  afterEach(() => {
    vi.restoreAllMocks();
    useExportAudio.getState().hide();
  });

  it("opens on ctrl/cmd+shift+e (not from a text field)", async () => {
    await setup();
    act(() => useExportAudio.getState().hide());
    const input = document.body.appendChild(document.createElement("input"));
    fireEvent.keyDown(input, { key: "E", ctrlKey: true, shiftKey: true });
    expect(useExportAudio.getState().open).toBe(false);
    input.remove();
    fireEvent.keyDown(document.body, { key: "E", ctrlKey: true, shiftKey: true });
    expect(useExportAudio.getState().open).toBe(true);
  });

  it("renders a mix, shows progress and downloads the result", async () => {
    const { mock, dialog } = await setup();
    pickOption(within(dialog).getByRole("combobox", { name: "format" }), { value: "Flac" });
    // FLAC has no float option.
    const depth = within(dialog).getByRole("combobox", { name: "bit depth" });
    fireEvent.click(depth);
    const list = document.getElementById(depth.getAttribute("aria-controls")!)!;
    expect(within(list).getByRole("option", { name: "32-bit float" })).toHaveAttribute("aria-disabled", "true");
    fireEvent.keyDown(depth, { key: "Escape" });
    fireEvent.click(within(dialog).getByRole("switch", { name: "normalize" }));
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "export" }));
    });
    expect(mock.renders()).toEqual([
      {
        range: { type: "Project" },
        format: { container: "Flac", bit_depth: "Int24", sample_rate: null },
        mode: { type: "Mix" },
        normalize: true,
        tail_seconds: 0,
        name: null,
      },
    ]);
    expect(within(dialog).getByRole("button", { name: "cancel export" })).toBeInTheDocument();
    await act(async () => {
      mock.tick(16);
    });
    expect(within(dialog).getByLabelText("export progress")).toHaveAttribute("value", "0.5");
    // Closed mid-render: the top bar shows the progress; a click reopens the dialog.
    act(() => useExportAudio.getState().hide());
    expect(screen.getByRole("button", { name: "export audio" })).toHaveTextContent("50%");
    fireEvent.click(screen.getByRole("button", { name: "export audio" }));
    expect(screen.getByRole("dialog", { name: "export audio" })).toBeInTheDocument();
    await act(async () => {
      mock.tick(16);
    });
    const done = await within(dialog).findByTestId("export-done");
    await waitFor(() => expect(within(done).getByRole("button", { name: "download again" })).toBeInTheDocument());
    expect(created).toHaveLength(1);
    expect(done).toHaveTextContent(".flac");
    // Closing releases the engine-side bytes.
    fireEvent.click(within(dialog).getByRole("button", { name: "close" }));
    await waitFor(() => expect(mock.sent.some((c) => c.domain === "Export" && c.command.type === "Release")).toBe(true));
  });

  it("releases a download only after it was pulled", async () => {
    const { mock, dialog } = await setup();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "export" }));
    });
    // Hold the chunk reads until the dialog was closed.
    let unblock!: () => void;
    const gate = new Promise<void>((r) => (unblock = r));
    const send = mock.send.bind(mock);
    mock.send = async (c, o) => {
      if (c.domain === "Export" && c.command.type === "ReadChunk") await gate;
      return send(c, o);
    };
    await act(async () => {
      mock.tick(16);
      mock.tick(16);
    });
    await within(dialog).findByTestId("export-done");
    // A new render would drop the bytes being pulled: Export waits for the downloads.
    expect(within(dialog).getByRole("button", { name: "export" })).toBeDisabled();
    fireEvent.click(within(dialog).getByRole("button", { name: "close" }));
    const order = () => mock.sent.flatMap((c) => (c.domain === "Export" ? [c.command.type] : []));
    expect(order()).not.toContain("Release");
    unblock();
    await waitFor(() => expect(order()).toContain("Release"));
    const o = order();
    expect(o.lastIndexOf("ReadChunk")).toBeLessThan(o.indexOf("Release"));
    expect(created).toHaveLength(1);
  });

  it("shows the stem semantics", async () => {
    const { dialog } = await setup();
    fireEvent.click(within(dialog).getByRole("switch", { name: "stems" }));
    expect(dialog).toHaveTextContent("solo is ignored");
    expect(within(dialog).queryByRole("note")).toBeNull();
  });

  it("exports stems of the chosen tracks and can be cancelled", async () => {
    const { mock, dialog } = await setup();
    fireEvent.click(within(dialog).getByRole("switch", { name: "stems" }));
    const list = within(dialog).getByRole("list", { name: "stem tracks" });
    const project = useProjectStore.getState().project!;
    const defaults = defaultStemTracks(project);
    expect(defaults.length).toBeGreaterThan(1);
    // Untick the first default track.
    const first = project.tracks[defaults[0]!]!;
    fireEvent.click(within(list).getByRole("switch", { name: first.name }));
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "export" }));
    });
    const req = mock.renders()[0]!;
    expect(req.mode).toEqual({ type: "Stems", tracks: defaults.slice(1) });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "cancel export" }));
    });
    expect(mock.sent.some((c) => c.domain === "Export" && c.command.type === "Cancel")).toBe(true);
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "export" })).toBeInTheDocument());
  });
});

describe("ExportDialog when the engine goes away (#587)", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    useExportAudio.getState().hide();
  });

  it("switching engines mid-render fails the job; the dialog can export again", async () => {
    const a = new SpyMock({ timers: "manual", seed: 5 });
    const b = new SpyMock({ timers: "manual", seed: 6 });
    let sw: ((t: MockTransport | null) => void) | null = null;
    const Switch = () => {
      const s = useTransportSwitch();
      useEffect(() => {
        sw = (t) => (t ? s!.switchToRemote(t) : s!.switchToLocal());
      });
      return null;
    };
    render(
      <TransportProvider transport={a}>
        <ExportDialog />
        <Switch />
      </TransportProvider>,
    );
    await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
    act(() => useExportAudio.getState().show());
    const dialog = await screen.findByRole("dialog", { name: "export audio" });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "export" }));
    });
    expect(within(dialog).getByRole("button", { name: "cancel export" })).toBeInTheDocument();

    // The connection to engine A goes; B takes over (A never reports the job again).
    act(() => sw!(b));
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "export" })).toBeEnabled());
    expect(dialog).toHaveTextContent("export interrupted");
    act(() => useExportAudio.getState().hide());
    expect(screen.queryByRole("button", { name: "export audio" })).toBeNull();
    b.dispose();
    a.dispose();
  });

  it("cancel frees the dialog even when the engine never confirms", async () => {
    const { mock, dialog } = await setup();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "export" }));
    });
    // An engine that lost the job: Cancel answers `Unit` and no `Cancelled` event follows.
    const send = mock.send.bind(mock);
    mock.send = (c, o) => (c.domain === "Export" && c.command.type === "Cancel" ? Promise.resolve({ type: "Unit" }) : send(c, o));
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "cancel export" }));
    });
    expect(within(dialog).getByRole("button", { name: "export" })).toBeEnabled();
    // The cancelled render finishing later doesn't bring the dialog back to it.
    await act(async () => {
      mock.tick(16);
      mock.tick(16);
    });
    expect(within(dialog).queryByTestId("export-done")).toBeNull();
    expect(within(dialog).getByRole("button", { name: "export" })).toBeEnabled();
  });
});

describe("export request", () => {
  it("builds requests and validates the form", () => {
    expect(formProblem({ ...DEFAULT_FORM, range: "selection" }, null)).toBe("export_audio.problem.no_selection");
    expect(formProblem({ ...DEFAULT_FORM, container: "Flac", bitDepth: "Float32" }, null)).toBe("export_audio.problem.flac_depth");
    expect(formProblem({ ...DEFAULT_FORM, stems: true }, null)).toBe("export_audio.problem.no_stems");
    expect(formProblem({ ...DEFAULT_FORM, container: "Mp3", rate: "96000" }, null)).toBe("export_audio.problem.mp3_rate");
    expect(formProblem({ ...DEFAULT_FORM, container: "Mp3", rate: "44100", bitDepth: "Float32" }, null)).toBeNull();
    const r = buildRequest(
      {
        ...DEFAULT_FORM,
        range: "selection",
        rate: "44100",
        tail: 99,
        name: "  Mix  ",
      },
      { start: 4, end: 12 },
    );
    expect(r).toEqual({
      range: { type: "Custom", start: 4, end: 12 },
      format: { container: "Wav", bit_depth: "Int24", sample_rate: 44100 },
      mode: { type: "Mix" },
      normalize: false,
      tail_seconds: 60,
      name: "Mix",
    });
  });
});

describe("export size estimate", () => {
  it("scales with length, rate, depth and files", () => {
    let n = 0;
    const project = createEmptyProject(() => `id${n++}`, "P", "p" as never);
    const form = {
      ...DEFAULT_FORM,
      range: "selection" as const,
      bitDepth: "Float32" as const,
      rate: "96000" as const,
    };
    // 20 min at 120 bpm = 2400 beats: 1200 s · 96 kHz · 2 ch · 4 B.
    const one = estimateBytes(form, project, { start: 0, end: 2400 });
    expect(one).toBeCloseTo(1200 * 96000 * 2 * 4);
    expect(one).toBeGreaterThan(LARGE_EXPORT_BYTES);
    expect(
      estimateBytes({ ...form, stems: true, tracks: ["a", "b"] }, project, {
        start: 0,
        end: 2400,
      }),
    ).toBeCloseTo(2 * one);
    expect(estimateBytes({ ...form, container: "Flac", bitDepth: "Int16" }, project, { start: 0, end: 4 })).toBeLessThan(LARGE_EXPORT_BYTES);
    // MP3: 320 kbps whatever the rate and depth.
    expect(estimateBytes({ ...form, container: "Mp3" }, project, { start: 0, end: 2400 })).toBeCloseTo((1200 * 320_000) / 8);
  });
});
