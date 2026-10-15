import { Fragment, type ReactNode } from "react";
import { useLocale } from "./hooks";
import { formatMessage } from "./runtime";
import type { MessageKey } from "./types";

type RichValues = Record<string, ReactNode | ((chunks: ReactNode[]) => ReactNode)>;

/**
 * A message with markup: `<Trans k="news.read_more" values={{ link: (c) => <a href={url}>{c}</a> }} />`
 * for `"…<link>read more</link>"`, or elements as arguments (`{ name: <strong>…</strong> }`).
 * Plain messages use `t`.
 */
export function Trans({ k, values }: { k: MessageKey; values?: RichValues }) {
  useLocale();
  const out = formatMessage(k, values);
  if (!Array.isArray(out)) return <>{out as ReactNode}</>;
  return (
    <>
      {out.map((part, i) => (
        <Fragment key={i}>{part as ReactNode}</Fragment>
      ))}
    </>
  );
}
