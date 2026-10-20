// The docs' commands: the same entries in the command palette and the project menu.
import { t } from "@/i18n";
import { useProjectScreen } from "@/features/project/screenStore";
import { openDocs } from "./open";
import { DOC_LINKS } from "./route";

export function docsCommands() {
  return [
    {
      id: "app:docs",
      group: "Help",
      label: t("docs.command.open"),
      keywords: "docs help manual guide documentation how tutorial learn reference",
      run: () => openDocs(),
    },
    {
      id: "app:docs-shortcuts",
      group: "Help",
      label: t("docs.command.shortcuts"),
      keywords: "docs help shortcuts keys reference cheat sheet",
      run: () => openDocs(DOC_LINKS.shortcuts),
    },
    {
      // The example beats are on the welcome screen (open one, or its tour).
      id: "app:examples",
      group: "Help",
      label: t("docs.command.examples"),
      keywords: "examples example beats demo loop template tutorial learn house hip-hop lo-fi bassline",
      run: () => useProjectScreen.getState().show("home"),
    },
  ];
}
