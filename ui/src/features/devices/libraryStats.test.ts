// The library's sort defaults to "most used"; a sort saved before that default keeps only a real choice.
import { afterEach, describe, expect, it } from "vitest";
import { load } from "./libraryStats";

const save = (v: unknown) => localStorage.setItem("eth-device-library", JSON.stringify(v));

afterEach(() => localStorage.removeItem("eth-device-library"));

describe("library sort default", () => {
  it("is most used with nothing saved", () => {
    expect(load().librarySort).toBe("MostUsed");
  });

  it("an old save's sort: the old default (name) becomes most used, another choice stays", () => {
    save({ sort: "Name" });
    expect(load().librarySort).toBe("MostUsed");
    save({ sort: "Recent" });
    expect(load().librarySort).toBe("Recent");
  });

  it("a sort chosen since is kept, name included", () => {
    save({ sort: "Recent", librarySort: "Name" });
    expect(load().librarySort).toBe("Name");
  });
});
