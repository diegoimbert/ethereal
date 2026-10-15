// The tours' text: catalog messages (docs/LOCALIZATION.md, keys `tutorials.*`), formatted
// when a bubble renders (never when the tour is defined), with `<b>…</b>` for emphasis.
import type { ReactNode } from "react";
import { Trans, type MessageKey } from "@/i18n";

type Values = Record<string, ReactNode | ((chunks: ReactNode[]) => ReactNode)>;

const RICH: Values = { b: (chunks: ReactNode[]) => <strong>{chunks}</strong> };

/** A message, with `<b>` as `<strong>`. */
export function Text({ k, values }: { k: MessageKey; values?: Values }) {
  return <Trans k={k} values={values ? { ...RICH, ...values } : RICH} />;
}

/** A paragraph of the bubble. */
export function P({ k, values }: { k: MessageKey; values?: Values }) {
  return (
    <p>
      <Text k={k} values={values} />
    </p>
  );
}

/** A list item of the bubble. */
export function Li({ k }: { k: MessageKey }) {
  return (
    <li>
      <Text k={k} />
    </li>
  );
}
