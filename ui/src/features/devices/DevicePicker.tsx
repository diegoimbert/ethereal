/**
 * A button that opens the device library (`DeviceLibrary`: search, keyboard navigation,
 * favourites, sort, All / Factory / Plugins and the "Plugin settings" footer) in a popover.
 * The one picker for every "add / choose a device or plugin" control that isn't a section's
 * "+" (a track's add menu, rack chains, drum pads, Replace plugin, the import resolver).
 * Filtering (`category`, `categories`, `accept`, `pluginsOnly`, `plugins`) is the caller's.
 */
import clsx from "clsx";
import { ChevronDown } from "lucide-react";
import type { ReactNode } from "react";
import { Popover } from "@/kit";
import { DeviceLibrary, type DeviceLibraryProps } from "./DeviceLibrary";
import type { LibraryDevicePayload } from "./libraryDevice";
import "./deviceLibrary.css";

export interface DevicePickerProps extends Omit<DeviceLibraryProps, "onPick" | "onClose"> {
  /** Accessible name of the trigger button. */
  "aria-label": string;
  /** The trigger's text (a placeholder, or the current choice). */
  children: ReactNode;
  className?: string;
  /** A row was picked (the popover is closed). */
  onPick(payload: LibraryDevicePayload): void;
  /** The footer's plugin settings were opened: close whatever surrounds the picker (a dialog). */
  onBeforeSettings?(): void;
  disabled?: boolean;
  /** Style the trigger text as a placeholder (nothing chosen). */
  placeholder?: boolean;
}

export function DevicePicker({
  "aria-label": label,
  children,
  className,
  onPick,
  onBeforeSettings,
  disabled,
  placeholder = false,
  ...library
}: DevicePickerProps) {
  return (
    <Popover
      aria-label={label}
      placement="bottom-start"
      trigger={(p) => (
        <button
          type="button"
          className={clsx("eth-select", "eth-select--sm", p["aria-expanded"] && "eth-select--open", className)}
          aria-label={label}
          disabled={disabled}
          {...p}
        >
          <span className={clsx("eth-select__value", placeholder && "eth-select__value--placeholder")}>{children}</span>
          <ChevronDown className="eth-select__chevron" aria-hidden />
        </button>
      )}
    >
      {(close) => (
        <DeviceLibrary
          {...library}
          onClose={() => {
            close();
            onBeforeSettings?.();
          }}
          onPick={(payload) => {
            close();
            onPick(payload);
          }}
        />
      )}
    </Popover>
  );
}
