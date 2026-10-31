import { describe, expect, it } from "vitest";
import { LEVELS, levelColor, levelLabel, levelRank, nextLevel } from "./levels";

describe("freshness levels", () => {
  it("orders the ladder from draft to confident", () => {
    expect(LEVELS).toEqual(["draft", "stable", "verified", "confident"]);
  });

  it("labels every level in Chinese", () => {
    expect(LEVELS.map(levelLabel)).toEqual(["草稿", "稳定", "已验证", "可信"]);
  });

  it("colors every level distinctly", () => {
    expect(LEVELS.map(levelColor)).toEqual(["default", "blue", "green", "gold"]);
  });

  it("ranks levels by ladder position starting at zero", () => {
    expect(LEVELS.map(levelRank)).toEqual([0, 1, 2, 3]);
  });

  it("promotes one step at a time and stops at confident", () => {
    expect(nextLevel("draft")).toBe("stable");
    expect(nextLevel("stable")).toBe("verified");
    expect(nextLevel("verified")).toBe("confident");
    expect(nextLevel("confident")).toBeNull();
  });
});
