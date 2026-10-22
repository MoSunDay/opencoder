// @vitest-environment jsdom
import "../../../testSetup";
import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import Labels from "../canvas3d/Labels";

describe("complete 3D relationship labels", () => {
  it.each([
    "主要归属外部协议边界，并通过已确认接口建立跨模块的依赖关系",
    "dependency_without_spaces_0123456789_abcdefghijklmnopqrstuvwxyz_ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "归属 🚀 API / source → target：a longer name with spaces",
    "第一行\n第二行原有内容\n\n最后一行",
    '<img src=x onerror="alert(1)">',
  ])("preserves the entire plain-text name and opens details: %s", (name) => {
    const click = vi.fn();
    const page = render(<Labels data={{ nodes: [], links: [{ id: "r", source: "a", target: "b", typeId: "rel", curvature: 0, rotation: 0 }] }}
      centers={[]} remaining={{}} entityTypeNames={{}} relationshipTypeNames={{ rel: name }} disabled={false}
      onNodeClick={vi.fn()} onEdgeClick={click} onExpand={vi.fn()} />);
    const label = page.container.querySelector('[data-edge-id="r"]')!;
    expect(label.textContent).toBe(name); expect(label.querySelector("img")).toBeNull();
    fireEvent.click(label); expect(click).toHaveBeenCalledWith("r");
  });
});

import { placeLabel } from "../canvas3d/labelLayout";
it("keeps relationship names outside node labels and inside narrow viewports", () => {
  const occupied = [{ x: 80, y: 80, width: 156, height: 52 }];
  const label = placeLabel({ x: 130, y: 100 }, 60, 26, occupied, { width: 350, height: 500 });
  expect(label.x + label.width <= 80 || label.x >= 236 || label.y + label.height <= 80 || label.y >= 132).toBe(true);
  const long = placeLabel({ x: 345, y: 495 }, 180, 90, occupied, { width: 350, height: 500 });
  expect(long.x + long.width).toBeLessThanOrEqual(346); expect(long.y + long.height).toBeLessThanOrEqual(496);
});
