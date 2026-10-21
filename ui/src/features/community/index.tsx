// The link to the ethereal discord community (the splash screen, the web's top band).
import type { SVGProps } from "react";
import { useT } from "@/i18n";
import { Button } from "@/kit";
import { openExternal } from "@/transport";

/** The discord invite (the only place it is written). */
export const DISCORD_INVITE_URL = "https://discord.gg/jSKkuqpQ2b";

/** The discord glyph in the kit's (lucide) line style: a 24 grid, 2 px round strokes. */
export function DiscordIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="lucide lucide-discord"
      aria-hidden="true"
      {...props}
    >
      <path d="M8.6 7.4C10.9 7 13.1 7 15.4 7.4L16 6.2C17.8 6.5 19.4 7.1 20.7 7.9C22.2 10.8 22.4 13.6 22 16.3C20.5 17.5 18.8 18.3 17.1 18.8L15.9 16.9C13.3 17.6 10.7 17.6 8.1 16.9L6.9 18.8C5.2 18.3 3.5 17.5 2 16.3C1.6 13.6 1.8 10.8 3.3 7.9C4.6 7.1 6.2 6.5 8 6.2Z" />
      <circle cx="9" cy="12.5" r="1" />
      <circle cx="15" cy="12.5" r="1" />
    </svg>
  );
}

export interface DiscordLinkProps {
  /**
   * "button": a full button, "join discord" (the splash screen); "label": a quiet ghost button,
   * "discord" (the web band).
   */
  variant?: "button" | "label";
  className?: string;
  /** Open the invite (injectable for tests); default: the browser (`openExternal`). */
  open?: (url: string) => void;
}

/** A button that opens the discord invite outside the app. */
export function DiscordLink({ variant = "label", className, open }: DiscordLinkProps) {
  const t = useT();
  const onClick = () => (open ?? openExternal)(DISCORD_INVITE_URL);
  if (variant === "button") {
    return (
      <Button size="sm" className={className} onClick={onClick} data-testid="discord-link">
        <DiscordIcon />
        {t("community.discord.join_button")}
      </Button>
    );
  }
  return (
    <Button size="sm" tone="ghost" className={className} title={t("community.discord.join")} onClick={onClick} data-testid="discord-link">
      <DiscordIcon />
      {t("community.discord.label")}
    </Button>
  );
}
