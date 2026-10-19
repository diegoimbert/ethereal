import { beforeEach, describe, expect, it } from "vitest";
import { drawerCanFullscreen, resetShell, useShellStore } from "./shell";

const shell = () => useShellStore.getState();

describe("drawer fullscreen", () => {
  beforeEach(() => resetShell());

  it("only the clip editor (piano roll / beat editor) can go fullscreen", () => {
    expect(drawerCanFullscreen("piano-roll")).toBe(true);
    expect(drawerCanFullscreen("warp")).toBe(false);
    expect(drawerCanFullscreen("automation")).toBe(false);
  });

  it("toggles on the open piano roll, and leaves the drawer's height alone", () => {
    shell().openDrawer("piano-roll");
    shell().setSize("bottom", 240);
    shell().setDrawerFullscreen(true);
    expect(shell().drawerFullscreen).toBe(true);
    expect(shell().bottom.size).toBe(240);
    shell().setDrawerFullscreen(false);
    expect(shell().drawerFullscreen).toBe(false);
    expect(shell().bottom.size).toBe(240);
  });

  it("does not enter when the drawer is closed, collapsed or shows another editor", () => {
    shell().setDrawerFullscreen(true);
    expect(shell().drawerFullscreen).toBe(false);
    shell().openDrawer("warp");
    shell().setDrawerFullscreen(true);
    expect(shell().drawerFullscreen).toBe(false);
    shell().openDrawer("piano-roll");
    shell().setCollapsed("bottom", true);
    shell().setDrawerFullscreen(true);
    expect(shell().drawerFullscreen).toBe(false);
  });

  it("leaves when the drawer switches to another editor, closes or collapses", () => {
    const enter = () => {
      shell().openDrawer("piano-roll");
      shell().setCollapsed("bottom", false);
      shell().setDrawerFullscreen(true);
      expect(shell().drawerFullscreen).toBe(true);
    };
    enter();
    shell().openDrawer("automation");
    expect(shell().drawerFullscreen).toBe(false);
    enter();
    shell().setOpen("bottom", false);
    expect(shell().drawerFullscreen).toBe(false);
    enter();
    shell().setCollapsed("bottom", true);
    expect(shell().drawerFullscreen).toBe(false);
  });

  it("stays on when the piano roll opens another clip, and is never remembered", () => {
    shell().openDrawer("piano-roll");
    shell().setDrawerFullscreen(true);
    shell().openDrawer("piano-roll");
    expect(shell().drawerFullscreen).toBe(true);
    expect(localStorage.getItem("eth.shell")).not.toContain("Fullscreen");
  });
});
