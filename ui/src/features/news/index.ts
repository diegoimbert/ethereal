// news: the owner's announcements, shown once after launch in a sandboxed modal (docs/NEWS.md).
import { t } from "@/i18n";
import { openNewsList } from "./store";

export { NewsRoot, NewsSettings } from "./NewsRoot";
export { closeNews, openNewsList, useNews, type NewsItem } from "./store";

export interface NewsCommand {
  id: string;
  group: string;
  label: string;
  keywords: string;
  run(): void;
}

/** "News…" in the command palette's Help group. */
export function newsCommands(): NewsCommand[] {
  return [
    {
      id: "app:news",
      group: "Help",
      label: t("menu.action.app_news"),
      keywords: "news announcements messages what's new updates author",
      run: () => openNewsList(),
    },
  ];
}
