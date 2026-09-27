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
  MockTransport,
  TransportProvider,
  type SendOptions,
} from "@/transport";
import { ExportDialog } from "./index";
import {
  buildRequest,
  DEFAULT_FORM,
  defaultStemTracks,
  formProblem,
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
    expect(
      mock.sent.some(
        (c) => c.domain === "Export" && c.command.type === "Release",
      ),
    ).toBe(true);
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
