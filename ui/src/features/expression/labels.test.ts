import { afterEach, describe, expect, it } from "vitest";
import type { ExpressionKind } from "@/generated";
import { resetI18nForTests, setLocale, tText } from "@/i18n/runtime";
import { kindKey, kindText, noteKindKey, noteKindText } from "./labels";

afterEach(() => resetI18nForTests());

describe("expression lane names", () => {
  const modWheel: ExpressionKind = { type: "Cc", controller: 1 };

  it("are messages, so a stroke's undo label follows the language", async () => {
    const draw = { id: "undo.expression.draw_lane", args: { lane: kindKey(modWheel) } };
    const edit = { id: "undo.expression.edit_lane", args: { lane: noteKindKey("Pitch") } };
    expect(kindText(modWheel)).toBe("cc 1 mod wheel");
    expect(tText(draw, "draw CC 1 mod wheel")).toBe("draw cc 1 mod wheel");
    expect(tText(edit, "edit note pitch")).toBe("edit note pitch");
    await setLocale("fr", { persist: false });
    expect(tText(draw, "draw CC 1 mod wheel")).toBe(`dessiner ${kindText(modWheel)}`);
    expect(tText(edit, "edit note pitch")).toBe(`modifier ${noteKindText("Pitch")}`);
    expect(kindText({ type: "Cc", controller: 3 })).toBe("cc 3");
  });
});
