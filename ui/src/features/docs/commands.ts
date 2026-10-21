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
      // The tutorials (guided tours) are the welcome screen's "get started" items.
      id: "app:tutorials",
      group: "Help",
      label: t("docs.command.tutorials"),
      keywords: "tutorials tutorial tour guided get started learn help first track",
      run: () => useProjectScreen.getState().show("home"),
    },
    {
      // The welcome (splash) screen: get started and the recent projects.
      id: "app:splash",
      group: "Help",
      label: t("docs.command.splash"),
      keywords: "splash welcome home start screen recent projects get started",
      run: () => useProjectScreen.getState().show("home"),
    },
  ];
}
