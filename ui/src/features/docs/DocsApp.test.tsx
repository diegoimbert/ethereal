import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { resetI18nForTests, setLocale } from "@/i18n/runtime";
import { loadDocs } from "./content";
import { DocsApp } from "./DocsApp";
import { useMemoryRouter } from "./router";
import { buildDocs } from "./model";
import { openDocs, useDocsOverlay } from "./open";
import { setKeyPlatform } from "./shortcuts";

const docs = buildDocs({
  "welcome/_section.md": "---\ntitle: welcome\norder: 1\nsummary: Start here.\n---\n",
  "welcome/intro.md": [
    "---\ntitle: what is ethereal\norder: 1\nsummary: The lead.\nplatform: all\n---",
    "Hello. Undo with [[action:edit.undo]]. See [projects](/docs/basics/projects#the-dot).",
    "## first part",
    "Some words about clips.",
    "## where next",
    "- [projects](/docs/basics/projects)",
  ].join("\n"),
  "basics/_section.md": "---\ntitle: the basics\norder: 2\n---\n",
  "basics/projects.md": [
    "---\ntitle: projects\norder: 1\nsummary: Saving.\nplatform: desktop\n---",
    "## the dot",
    "The unsaved dot.",
    "## where next",
    "Nothing.",
  ].join("\n"),
});

function Harness({ path = "", onClose }: { path?: string; onClose?: () => void }) {
  const [initial] = useState({ path, anchor: "" });
  const router = useMemoryRouter(initial);
  return <DocsApp router={router} docs={docs} onClose={onClose} />;
}

afterEach(() => {
  useDocsOverlay.setState({ open: false, path: "", anchor: "", request: 0 });
  vi.restoreAllMocks();
});

describe("DocsApp", () => {
  it("starts on the start page with every section", () => {
    render(<Harness />);
    const home = screen.getByTestId("docs-home");
    expect(within(home).getByText("welcome")).toBeInTheDocument();
    expect(within(home).getByText("the basics")).toBeInTheDocument();
    expect(within(home).getByText("start here")).toBeInTheDocument();
  });

  it("shows a page with its lead, badges, contents, shortcuts and pager; links navigate", () => {
    setKeyPlatform("pc");
    render(<Harness path="welcome/intro" />);
    const page = screen.getByTestId("docs-page");
    expect(page).toHaveAttribute("data-path", "welcome/intro");
    expect(within(page).getByText("The lead.")).toBeInTheDocument();
    expect(within(page).getByText("Ctrl+Z")).toBeInTheDocument();
    expect(within(page).getByRole("heading", { level: 2, name: /first part/ })).toHaveAttribute("id", "first-part");
    // Next page, in the pager.
    fireEvent.click(within(page).getByText("projects", { selector: ".eth-docs-pager__title" }));
    const next = screen.getByTestId("docs-page");
    expect(next).toHaveAttribute("data-path", "basics/projects");
    expect(within(next).getAllByText("desktop").length).toBeGreaterThan(0);
  });

  it("switches the keys between mac and windows / linux", () => {
    setKeyPlatform("pc");
    render(<Harness path="welcome/intro" />);
    fireEvent.click(screen.getByRole("radio", { name: "mac" }));
    expect(screen.getByText("⌘Z")).toBeInTheDocument();
    setKeyPlatform("pc");
  });

  it("the nav marks the current page; sections fold", () => {
    render(<Harness path="basics/projects" />);
    const nav = screen.getByTestId("docs-nav");
    expect(within(nav).getByText("projects")).toHaveAttribute("aria-current", "page");
    const welcome = within(nav).getByRole("button", { name: /welcome/ });
    expect(welcome).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(welcome);
    expect(welcome).toHaveAttribute("aria-expanded", "false");
    expect(within(nav).queryByText("what is ethereal")).toBeNull();
  });

  it("searches and opens a result with the keyboard", () => {
    render(<Harness />);
    const input = screen.getByRole("combobox", { name: "search the docs" });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "unsaved" } });
    const results = screen.getByRole("listbox", { name: "search results" });
    expect(within(results).getAllByRole("option")[0]).toHaveTextContent("the basics › projects › the dot");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(screen.getByTestId("docs-page")).toHaveAttribute("data-path", "basics/projects");
  });

  it("says when nothing is found, and when a page doesn't exist", () => {
    render(<Harness path="nope/nothing" />);
    expect(screen.getByTestId("docs-missing")).toBeInTheDocument();
    const input = screen.getByRole("combobox", { name: "search the docs" });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "zzzz" } });
    expect(screen.getByText("nothing found")).toBeInTheDocument();
  });

  it("a section path opens its first page", () => {
    render(<Harness path="basics" />);
    expect(screen.getByTestId("docs-page")).toHaveAttribute("data-path", "basics/projects");
  });

  it("over the app: a dialog that closes with the button or Escape", () => {
    const onClose = vi.fn();
    render(<Harness path="welcome/intro" onClose={onClose} />);
    expect(screen.getByRole("dialog", { name: "documentation" })).toBeInTheDocument();
    fireEvent.keyDown(screen.getByTestId("docs-page"), { key: "Escape" });
    fireEvent.click(screen.getByTestId("docs-close"));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});

describe("the docs in the UI's language (#667)", () => {
  // Parsing every English and French page takes a while on a loaded machine: do it once, up front.
  beforeAll(() => loadDocs("fr"), 30_000);
  afterEach(() => resetI18nForTests());

  function Real({ path }: { path: string }) {
    const [initial] = useState({ path, anchor: "" });
    return <DocsApp router={useMemoryRouter(initial)} />;
  }

  it("shows a translated page in French, keeping its anchors", async () => {
    await setLocale("fr", { persist: false });
    render(<Real path="welcome/what-is-ethereal" />);
    expect(await screen.findByRole("heading", { level: 1, name: /qu’est-ce qu’ethereal/ }, { timeout: 10_000 })).toBeInTheDocument();
    const page = screen.getByTestId("docs-page");
    expect(within(page).getByRole("heading", { level: 2, name: /trois façons/ })).toHaveAttribute("id", "three-ways-to-run-it");
    expect(screen.queryByTestId("docs-untranslated")).toBeNull();
  });

  it("says when a page isn't translated, and marks it as English", async () => {
    await setLocale("fr", { persist: false });
    render(<Harness path="welcome/intro" />);
    expect(await screen.findByTestId("docs-untranslated", {}, { timeout: 10_000 })).toHaveTextContent("cette page n’est pas encore traduite");
    expect(screen.getByTestId("docs-page").querySelector("article")).toHaveAttribute("lang", "en");
  });

  it("in English, no notice", () => {
    render(<Harness path="welcome/intro" />);
    expect(screen.queryByTestId("docs-untranslated")).toBeNull();
  });
});

describe("openDocs", () => {
  it("opens a new tab at /docs on the web", () => {
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    openDocs("reference/keyboard-shortcuts", "", { inApp: false });
    expect(open).toHaveBeenCalledWith("/docs/reference/keyboard-shortcuts", "_blank", "noopener");
  });

  it("opens over the app in the desktop app", () => {
    act(() => openDocs("basics/projects", "the-dot", { inApp: true }));
    expect(useDocsOverlay.getState()).toMatchObject({ open: true, path: "basics/projects", anchor: "the-dot", request: 1 });
  });
});
