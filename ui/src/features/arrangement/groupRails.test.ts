import { describe, expect, it } from "vitest";
import type { Track } from "@/generated";
import { groupAncestors } from "./groupRails";

const t = (id: string, parent: string | null) => ({ id, parent }) as unknown as Track;

describe("groupAncestors", () => {
  const tracks: Record<string, Track> = { g1: t("g1", null), g2: t("g2", "g1"), a: t("a", "g2"), b: t("b", null) };
  it("lists the enclosing groups, outermost first", () => {
    expect(groupAncestors(tracks.a!, tracks).map((g) => g.id)).toEqual(["g1", "g2"]);
    expect(groupAncestors(tracks.g2!, tracks).map((g) => g.id)).toEqual(["g1"]);
  });
  it("is empty at the top level", () => {
    expect(groupAncestors(tracks.b!, tracks)).toEqual([]);
  });
  it("stops at a missing parent", () => {
    expect(groupAncestors(t("c", "gone"), tracks)).toEqual([]);
  });
});
