import clsx from "clsx";
import { Search, X } from "lucide-react";
import type { KeyboardEvent, Ref } from "react";
import { useT } from "@/i18n";
import { TextInput } from "./fields";

export interface SearchFieldProps {
  value: string;
  onChange(value: string): void;
  /** Its accessible name and placeholder. */
  label: string;
  onKeyDown?(e: KeyboardEvent<HTMLInputElement>): void;
  autoFocus?: boolean;
  inputRef?: Ref<HTMLInputElement>;
  className?: string;
}

/**
 * A search field: a magnifier inside on the left, a clear button on the right once something
 * is typed (no autocorrect). The device library's, the sample picker's, the sidechain picker's.
 */
export function SearchField({ value, onChange, label, onKeyDown, autoFocus, inputRef, className }: SearchFieldProps) {
  const t = useT();
  return (
    <span className={clsx("eth-search-field", className)}>
      <Search className="eth-search-field__icon" aria-hidden />
      <TextInput
        ref={inputRef}
        size="sm"
        type="search"
        className="eth-search-field__input"
        aria-label={label}
        placeholder={label}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        autoFocus={autoFocus}
      />
      {value && (
        <button type="button" className="eth-search-field__clear" aria-label={t("kit.search.clear")} onClick={() => onChange("")}>
          <X aria-hidden />
        </button>
      )}
    </span>
  );
}
