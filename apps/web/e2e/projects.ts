// Shared e2e gesture: create (and switch to) a new project through the project screen.
import type { Page } from "@playwright/test";

export async function newProject(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name: "Projects" }).click();
  const screen = page.getByRole("dialog", { name: "Projects" });
  await screen.getByRole("button", { name: "New project" }).click();
  await screen.getByLabel("New project name").fill(name);
  await screen.getByRole("button", { name: "Create" }).click();
}
