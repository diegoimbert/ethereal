import clsx from "clsx";
import { MoreHorizontal } from "lucide-react";
import type { ReactNode } from "react";
import { Button, type ButtonProps } from "./Button";
import { Popover, type Placement, type TriggerProps } from "./overlays";

/** A button's attachment: a small secondary button joined to its right edge. */
export interface ButtonAttachment {
  /** Accessible name and tooltip, e.g. "quantize settings". */
  label: string;
  /** Its icon; default "…" (more). */
  icon?: ReactNode;
  /** A popover it opens (its content; a function gets `close`). */
  popover?: ReactNode | ((close: () => void) => ReactNode);
  /** The popover's placement; default "bottom-start". */
  placement?: Placement;
  /** Without a popover: what a click does. */
  onClick?: () => void;
  disabled?: boolean;
  testId?: string;
}

export interface AttachedButtonProps extends ButtonProps {
  attachment: ButtonAttachment;
}

/**
 * A button with an attachment: a secondary button joined to its right ("…" by default) that
 * opens a popover (e.g. the action's settings) or does its own thing. The main button takes
 * every `Button` prop; both share its size and tone.
 */
export function AttachedButton({ attachment: a, className, size = "md", tone, variant, ...main }: AttachedButtonProps) {
  const piece = (trigger?: TriggerProps) => (
    <Button
      size={size}
      tone={tone}
      variant={variant}
      className="eth-attached__piece"
      aria-label={a.label}
      title={a.label}
      disabled={a.disabled}
      data-testid={a.testId}
      onClick={a.onClick}
      {...trigger}
    >
      {a.icon ?? <MoreHorizontal aria-hidden />}
    </Button>
  );
  return (
    <span className={clsx("eth-attached", className)}>
      <Button size={size} tone={tone} variant={variant} className="eth-attached__main" {...main} />
      {a.popover === undefined ? (
        piece()
      ) : (
        <Popover aria-label={a.label} placement={a.placement ?? "bottom-start"} trigger={piece}>
          {a.popover}
        </Popover>
      )}
    </span>
  );
}
