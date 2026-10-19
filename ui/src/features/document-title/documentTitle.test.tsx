import { act, cleanup, render, renderHook } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Patch, Project } from "@/generated";
import { EMPTY_HISTORY, useProjectStore, useViewerStore } from "@/state";
import { createDemoProject } from "@/transport/mock/demoProject";
import { DocsApp } from "@/features/docs/DocsApp";
import { buildDocs } from "@/features/docs/model";
import { useMemoryRouter } from "@/features/docs/router";
import { useProjectScreen } from "@/features/project/screenStore";
import { tabTitle, useDocumentTitle } from "./index";

const store = () => useProjectStore.getState();
const named = (name: string): Project => {
  const p = createDemoProject();
  return { ...p, settings: { ...p.settings, name } };
};
const rename = (revision: number, name: string): Patch => ({
  revision,
  changes: [{ type: "Settings", settings: { ...store().project!.settings, name } }],
  history: EMPTY_HISTORY,
});

beforeEach(() => {
  store().reset();
  useViewerStore.getState().setViewer(null);
  useProjectScreen.getState().hide();
  document.title = "ethereal";
});
afterEach(cleanup);

describe("tabTitle", () => {
  it("is `ethereal - <part>`, plain `ethereal` without one, with `• ` while unsaved", () => {
    expect(tabTitle("My song")).toBe("ethereal - My song");
    expect(tabTitle("  ")).toBe("ethereal");
    expect(tabTitle(null)).toBe("ethereal");
    expect(tabTitle("My song", true)).toBe("• ethereal - My song");
  });
});

describe("useDocumentTitle", () => {
  it("is plain `ethereal` with no project open (the project picker)", () => {
    renderHook(() => useDocumentTitle());
    expect(document.title).toBe("ethereal");
  });

  it("names the open project, follows renames (local or a collaborator's, both are patches) and marks unsaved changes", () => {
    act(() => store().loadProject(named("My song")));
    renderHook(() => useDocumentTitle());
    expect(document.title).toBe("ethereal - My song");
    act(() => void store().applyPatch(rename(1, "Night drive")));
    expect(document.title).toBe("ethereal - Night drive");
    act(() => store().setDirty(true));
    expect(document.title).toBe("• ethereal - Night drive");
    act(() => store().setDirty(false));
    expect(document.title).toBe("ethereal - Night drive");
  });

  it("follows a project switch (also Back / Forward) and closing the project", () => {
    act(() => store().loadProject(named("One")));
    renderHook(() => useDocumentTitle());
    act(() => store().loadProject(named("Two")));
    expect(document.title).toBe("ethereal - Two");
    act(() => store().clearProject());
    expect(document.title).toBe("ethereal");
  });

  it("is plain `ethereal` while the project picker shows (over the launch draft), not while saving as", () => {
    act(() => store().loadProject(named("launch draft")));
    renderHook(() => useDocumentTitle());
    act(() => useProjectScreen.getState().show());
    expect(document.title).toBe("ethereal");
    act(() => useProjectScreen.getState().setMode("saveAs"));
    expect(document.title).toBe("ethereal - launch draft");
    act(() => useProjectScreen.getState().hide());
    expect(document.title).toBe("ethereal - launch draft");
  });

  it("names the shared project in the viewer, never as unsaved", () => {
    act(() => {
      store().loadProject(named("internal name"));
      store().setDirty(true);
      useViewerStore.getState().setViewer({ name: "Shared song" });
    });
    renderHook(() => useDocumentTitle());
    expect(document.title).toBe("ethereal - Shared song");
  });

  it("goes back to plain `ethereal` once unmounted (an error screen replaces the app)", () => {
    act(() => store().loadProject(named("My song")));
    const { unmount } = renderHook(() => useDocumentTitle());
    unmount();
    expect(document.title).toBe("ethereal");
  });
});

const docs = buildDocs({
  "basics/_section.md": "---\ntitle: the basics\norder: 1\n---\n",
  "basics/clips.md": "---\ntitle: clips\norder: 1\nsummary: Clips.\nplatform: all\n---\nSome words.",
});

function Docs({ path }: { path: string }) {
  const [initial] = useState({ path, anchor: "" });
  return <DocsApp router={useMemoryRouter(initial)} docs={docs} />;
}

describe("the pages outside the editor", () => {
  it("the docs: `ethereal - documentation`, the page's name in front on a page", () => {
    const { unmount } = render(<Docs path="" />);
    expect(document.title).toBe("ethereal - documentation");
    unmount();
    render(<Docs path="basics/clips" />);
    expect(document.title).toBe("ethereal - clips · documentation");
  });
});
