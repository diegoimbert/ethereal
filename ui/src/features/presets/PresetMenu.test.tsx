import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Device, DeviceId } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, type MockTransport } from "@/transport";
import { ContextMenuHost } from "@/kit";
import { renderWithMock, resetStores, store } from "@/features/mixer/testUtils";
import { PresetMenu } from "./index";
import { findByName, parseTags, useCurrentPresets } from "./model";

let mock: MockTransport | undefined;
afterEach(() => {
  resetStores(mock);
  mock = undefined;
  useCurrentPresets.setState({ current: {} });
});

function Live({ id }: { id: DeviceId }) {
  const device = useProjectStore((s) => s.project?.devices[id]);
  return device ? <PresetMenu device={device} /> : null;
}

const synth = (): Device => Object.values(store().project!.devices).find((d) => d.kind.type === "Builtin" && d.kind.device.type === "Synth")!;

async function renderSynth() {
  mock = await renderWithMock(
    <>
      <Probe />
      <ContextMenuHost />
    </>,
  );
  return synth();
}

function Probe() {
  useProjectStore((s) => s.project);
  const p = store().project;
  return p ? <Live id={synth().id} /> : null;
}

const trigger = (name = "Synth") => screen.findByRole("button", { name: `Presets for ${name}` });

describe("PresetMenu", () => {
  it("lists factory presets and loads one (one undo step)", async () => {
    const d = await renderSynth();
    fireEvent.click(await trigger());
    const panel = await screen.findByRole("dialog", {
      name: "Presets for Synth",
    });
    const factory = await within(panel).findByRole("region", {
      name: "Factory presets",
    });
    expect(
      within(factory)
        .getAllByRole("button")
        .map((b) => b.querySelector(".eth-presets__name")?.textContent),
    ).toEqual(["Glass Keys", "Pluck", "Soft Pad", "Square Lead", "Sub Bass"]);
    // Search filters by name and tags.
    fireEvent.change(within(panel).getByRole("textbox", { name: "Search presets" }), { target: { value: "bass" } });
    await waitFor(() => expect(within(panel).getAllByRole("button", { name: /Sub Bass/ })).toHaveLength(1));
    await act(async () => {
      fireEvent.click(within(panel).getByRole("button", { name: /Sub Bass/ }));
    });
    await waitFor(() => expect(store().project!.devices[d.id]!.params[1]).toBe(-12));
    expect(await trigger()).toHaveTextContent("Sub Bass");
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    expect(store().project!.devices[d.id]!.params[1]).toBe(d.params[1]);
  });

  it("saves, then replaces, renames and deletes a user preset", async () => {
    const d = await renderSynth();
    fireEvent.click(await trigger());
    await act(async () => {
      fireEvent.click(await screen.findByRole("button", { name: "Save preset…" }));
    });
    const dialog = await screen.findByRole("dialog", {
      name: "Save preset for Synth",
    });
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Preset name" }), { target: { value: "My Lead" } });
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Preset tags" }), { target: { value: "Lead, bright" } });
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Save preset for Synth" })).toBeNull());
    expect(await trigger()).toHaveTextContent("My Lead");

    // The name exists now: the dialog offers to replace it.
    fireEvent.click(await trigger());
    await act(async () => {
      fireEvent.click(await screen.findByRole("button", { name: "Save preset…" }));
    });
    const again = await screen.findByRole("dialog", {
      name: "Save preset for Synth",
    });
    expect(within(again).getByRole("textbox", { name: "Preset name" })).toHaveValue("My Lead");
    expect(within(again).getByRole("textbox", { name: "Preset tags" })).toHaveValue("bright, lead");
    expect(await within(again).findByRole("button", { name: "Replace" })).toBeEnabled();
    fireEvent.click(within(again).getByRole("button", { name: "Cancel" }));

    // Rename through the user row's menu.
    fireEvent.click(await trigger());
    const user = await screen.findByRole("region", { name: "User presets" });
    fireEvent.click(within(user).getByRole("button", { name: "More actions for My Lead" }));
    await act(async () => {
      fireEvent.click(await screen.findByRole("menuitem", { name: "Rename…" }));
    });
    const rename = await screen.findByRole("dialog", {
      name: "Rename “My Lead”",
    });
    fireEvent.change(within(rename).getByRole("textbox", { name: "Preset name" }), { target: { value: "Bright Lead" } });
    await act(async () => {
      fireEvent.click(within(rename).getByRole("button", { name: "Rename" }));
    });
    await waitFor(() => expect(useCurrentPresets.getState().current[d.id]?.name).toBe("Bright Lead"));

    // Delete.
    fireEvent.click(await trigger());
    const user2 = await screen.findByRole("region", { name: "User presets" });
    fireEvent.click(
      within(user2).getByRole("button", {
        name: "More actions for Bright Lead",
      }),
    );
    await act(async () => {
      fireEvent.click(await screen.findByRole("menuitem", { name: "Delete Preset…" }));
    });
    const del = await screen.findByRole("dialog", {
      name: "Delete “Bright Lead”?",
    });
    await act(async () => {
      fireEvent.click(within(del).getByRole("button", { name: "Delete" }));
    });
    await waitFor(() => expect(useCurrentPresets.getState().current[d.id]).toBeUndefined());
    const list = await mock!.send(
      cmd("Preset", {
        type: "List",
        device: { type: "Builtin", device: "Synth" },
        text: null,
      }),
    );
    expect(list.type === "Presets" && list.presets.some((p) => p.preset.source === "User")).toBe(false);
  });
});

describe("preset model", () => {
  it("parses tags and finds user presets by name", () => {
    expect(parseTags(" Pad, warm ,pad,, ")).toEqual(["pad", "warm"]);
    const user = {
      preset: { source: "User" as const, id: "synth/a.etherpreset" },
      name: "Warm",
      device: { type: "Builtin" as const, device: "Synth" as const },
      meta: { tags: [], author: null, description: null },
    };
    const factory = {
      ...user,
      preset: { source: "Factory" as const, id: "synth/warm" },
    };
    expect(findByName([factory, user], "warm")).toBe(user);
    expect(findByName([factory], "warm")).toBeUndefined();
    expect(findByName([user], "warm", user.preset)).toBeUndefined();
  });
});
