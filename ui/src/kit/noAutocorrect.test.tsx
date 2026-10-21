// Search fields never get macOS's autocorrect, autocapitalization or spellcheck.
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { TextInput } from "./fields";

describe("search fields", () => {
  it("turn off autocorrect, autocapitalization, spellcheck and autocomplete", () => {
    render(<TextInput type="search" aria-label="find" />);
    const field = screen.getByRole("searchbox", { name: "find" });
    expect(field).toHaveAttribute("autocorrect", "off");
    expect(field).toHaveAttribute("autocapitalize", "off");
    expect(field).toHaveAttribute("spellcheck", "false");
    expect(field).toHaveAttribute("autocomplete", "off");
  });

  it("leave other text fields as they are", () => {
    render(<TextInput aria-label="name" />);
    expect(screen.getByRole("textbox", { name: "name" })).not.toHaveAttribute("autocorrect");
  });
});
