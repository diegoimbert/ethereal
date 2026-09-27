import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, ExportRequest, ReplyValue } from "@/generated";
import { useProjectStore } from "@/state";
import {
  createEmptyProject,
  MockTransport,
  TransportProvider,
  type SendOptions,
} from "@/transport";
import { ExportDialog } from "./index";
import {
  buildRequest,
  DEFAULT_FORM,
  defaultStemTracks,
  estimateBytes,
  formProblem,
  LARGE_EXPORT_BYTES,
} from "./request";

class SpyMock extends MockTransport {
  sent: Command[] = [];
  override send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push(command);
    return super.send(command, opts);
  }
  renders(): ExportRequest[] {
    return this.sent.flatMap((c) =>
      c.domain === "Export" && c.command.type === "Render"
        ? [c.command.request]
        : [],
    );
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
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Export audio" }),
    ).not.toBeDisabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Export audio" }));
  const dialog = await screen.findByRole("dialog", { name: "Export audio" });
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
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(
      () => undefined,
    );
  });
  afterEach(() => vi.restoreAllMocks());

  it("renders a mix, shows progress and downloads the result", async () => {
    const { mock, dialog } = await setup();
    fireEvent.change(within(dialog).getByLabelText("Format"), {
      target: { value: "Flac" },
    });
    // FLAC has no float option.
    const float = within(dialog).getByRole("option", {
      name: "32-bit float",
    }) as HTMLOptionElement;
    expect(float.disabled).toBe(true);
    fireEvent.click(within(dialog).getByRole("switch", { name: "Normalize" }));
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Export" }));
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
    expect(
      within(dialog).getByRole("button", { name: "Cancel export" }),
    ).toBeInTheDocument();
    await act(async () => {
      mock.tick(16);
    });
    expect(within(dialog).getByLabelText("Export progress")).toHaveAttribute(
      "value",
      "0.5",
    );
    expect(
      screen.getByRole("button", { name: "Export audio" }),
    ).toHaveTextContent("50%");
    await act(async () => {
      mock.tick(16);
    });
    const done = await within(dialog).findByTestId("export-done");
    await waitFor(() =>
      expect(
        within(done).getByRole("button", { name: "Download again" }),
      ).toBeInTheDocument(),
    );
    expect(created).toHaveLength(1);
    expect(done).toHaveTextContent(".flac");
    // Closing releases the engine-side bytes.
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    await waitFor(() =>
      expect(
        mock.sent.some(
          (c) => c.domain === "Export" && c.command.type === "Release",
        ),
      ).toBe(true),
    );
  });

  it("releases a download only after it was pulled", async () => {
    const { mock, dialog } = await setup();
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Export" }));
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
    expect(
      within(dialog).getByRole("button", { name: "Export" }),
    ).toBeDisabled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    const order = () =>
      mock.sent.flatMap((c) => (c.domain === "Export" ? [c.command.type] : []));
    expect(order()).not.toContain("Release");
    unblock();
    await waitFor(() => expect(order()).toContain("Release"));
    const o = order();
    expect(o.lastIndexOf("ReadChunk")).toBeLessThan(o.indexOf("Release"));
    expect(created).toHaveLength(1);
  });

  it("shows the stem semantics", async () => {
    const { dialog } = await setup();
    fireEvent.click(within(dialog).getByRole("switch", { name: "Stems" }));
    expect(dialog).toHaveTextContent("Solo is ignored");
    expect(within(dialog).queryByRole("note")).toBeNull();
  });

  it("exports stems of the chosen tracks and can be cancelled", async () => {
    const { mock, dialog } = await setup();
    fireEvent.click(within(dialog).getByRole("switch", { name: "Stems" }));
    const list = within(dialog).getByRole("list", { name: "Stem tracks" });
    const project = useProjectStore.getState().project!;
    const defaults = defaultStemTracks(project);
    expect(defaults.length).toBeGreaterThan(1);
    // Untick the first default track.
    const first = project.tracks[defaults[0]!]!;
    fireEvent.click(within(list).getByRole("switch", { name: first.name }));
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Export" }));
    });
    const req = mock.renders()[0]!;
    expect(req.mode).toEqual({ type: "Stems", tracks: defaults.slice(1) });
    await act(async () => {
      fireEvent.click(
        within(dialog).getByRole("button", { name: "Cancel export" }),
      );
    });
    expect(
      mock.sent.some(
        (c) => c.domain === "Export" && c.command.type === "Cancel",
      ),
    ).toBe(true);
    await waitFor(() =>
      expect(
        within(dialog).getByRole("button", { name: "Export" }),
      ).toBeInTheDocument(),
    );
  });
});

describe("export request", () => {
  it("builds requests and validates the form", () => {
    expect(formProblem({ ...DEFAULT_FORM, range: "selection" }, null)).toMatch(
      /time range/,
    );
    expect(
      formProblem(
        { ...DEFAULT_FORM, container: "Flac", bitDepth: "Float32" },
        null,
      ),
    ).toMatch(/FLAC/);
    expect(formProblem({ ...DEFAULT_FORM, stems: true }, null)).toMatch(
      /at least one/,
    );
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
    expect(
      estimateBytes(
        { ...form, container: "Flac", bitDepth: "Int16" },
        project,
        { start: 0, end: 4 },
      ),
    ).toBeLessThan(LARGE_EXPORT_BYTES);
  });
});
