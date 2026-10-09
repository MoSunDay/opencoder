// @vitest-environment jsdom
import "../../../testSetup";
import { describe, expect, it } from "vitest";
import { relationshipLabel } from "../canvas/relationshipLabel";

const measure = (text: string) => Array.from(text).reduce((width, character) => width + (/[^\x00-\x7F]/.test(character) ? 13 : 7), 0);

describe("complete relationship labels", () => {
  it.each([
    "主要归属外部协议边界，并通过已确认接口建立跨业务域的依赖关系",
    "dependency_without_spaces_0123456789_abcdefghijklmnopqrstuvwxyz_ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "归属 🚀 API / source → target：a longer name with spaces",
    "第一行\n第二行原有内容\n\n最后一行",
  ])("preserves every character and reserves enough space for %s", (name) => {
    const label = relationshipLabel(name, measure);
    const lines = label.text.split("\n");
    expect(lines.join("")).toBe(name.replaceAll("\n", ""));
    expect(label.text).not.toContain("...");
    expect(lines.every((line) => measure(line) <= 180)).toBe(true);
    expect(label.width).toBeGreaterThanOrEqual(Math.max(...lines.map(measure)) + 12);
    expect(label.height).toBe(lines.length * 20 + 12);
  });
});
