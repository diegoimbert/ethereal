import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ConfirmDialog } from "@/kit";
import { PluginListSettings } from "./PluginListSettings";
import { usePluginStore } from "./pluginStore";
import { VERB, VST_VERB, EQ } from "./testing";
import { usePluginVisibility } from "./visibility";

afterEach(() => {
  localStorage.removeItem("eth.plugins.visibility");
  usePluginVisibility.getState().reload();
  usePluginStore.getState().reset();
});

describe("Settings > Plugins > Plugin list", () => {
  it("turns one format per plugin off and on, picks the preferred format, and remembers it", () => {
    act(() => usePluginStore.getState().setPlugins([VERB, VST_VERB, EQ]));
    render(<PluginListSettings />);
    expect(screen.getByText("2 of 3 plugins listed.")).toBeInTheDocument();
    const toggle = screen.getByRole("switch", { name: "show each plugin once" });
    expect(toggle).toBeChecked();
    fireEvent.click(toggle);
    expect(screen.getByText("3 of 3 plugins listed.")).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "preferred format" })).toBeDisabled();
    expect(JSON.parse(localStorage.getItem("eth.plugins.visibility")!)).toMatchObject({ dedupe: false, preferred: "Clap" });
  });

  it("lists the hidden plugins: narrow one to a format, show it again, reset after asking", () => {
    act(() => void usePluginVisibility.getState().hide({ name: "Verb", vendor: "Acme", format: null }));
    render(
      <>
        <PluginListSettings />
        <ConfirmDialog />
      </>,
    );
    const rules = screen.getByRole("list", { name: "hidden plugins" });
    expect(within(rules).getByText("Verb")).toBeInTheDocument();
    expect(within(rules).getByRole("combobox", { name: "which formats of Verb are hidden" })).toHaveTextContent("every format");
    fireEvent.click(screen.getByRole("button", { name: "reset" }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "reset the plugin list?" })).getByRole("button", { name: "reset" }));
    expect(usePluginVisibility.getState().rules).toEqual([]);
    expect(screen.queryByRole("list", { name: "hidden plugins" })).toBeNull();
    act(() => void usePluginVisibility.getState().hide({ name: "Verb", vendor: "Acme", format: "Vst3" }));
    fireEvent.click(screen.getByRole("button", { name: "show Verb again" }));
    expect(usePluginVisibility.getState().rules).toEqual([]);
  });
});
