// @vitest-environment jsdom
import "../../../../test/setup-dom.js";
import { act, fireEvent, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import GraphCanvas from "../GraphCanvas";

const callbacks = vi.hoisted(() => ({ ready: undefined as undefined | ((graph: unknown) => void), height: 0 }));
vi.mock("@ant-design/graphs", () => ({
  FlowDirectionGraph: ({ onReady, height }: { onReady: (graph: unknown) => void; height: number }) => {
    callbacks.ready = onReady; callbacks.height = height; return null;
  },
}));

describe("graph lifecycle", () => {
  it("ignores a delayed ready callback for a destroyed graph and binds live handlers once", () => {
    const data = { nodes: ["a", "b"].map((id) => ({ id, env_num: 1, entity_type_id: "type", name: id, description: "", revision: 1, is_deleted: false })), edges: [] };
    const onNodeClick = vi.fn();
    const onEdgeClick = vi.fn();
    render(<GraphCanvas data={data} centerIds={["a", "b"]} relationshipTypeNames={{}} onNodeClick={onNodeClick} onEdgeClick={onEdgeClick} />);
    const stale = { destroyed: true, on: vi.fn(), setElementState: vi.fn().mockResolvedValue(undefined) };
    act(() => callbacks.ready?.(stale));
    expect(stale.on).not.toHaveBeenCalled();
    expect(stale.setElementState).not.toHaveBeenCalled();
    const live = { ...stale, destroyed: false };
    act(() => { callbacks.ready?.(live); callbacks.ready?.(live); });
    expect(live.on).toHaveBeenCalledTimes(3);
    expect(live.setElementState).toHaveBeenLastCalledWith({ a: ["selected"], b: ["selected"] }, false);
    live.on.mock.calls[0][1]({ target: { id: "a" } });
    expect(onNodeClick).toHaveBeenCalledWith("a");
  });

  it("uses the remaining viewport height and updates on resize", () => {
    const previousHeight = window.innerHeight;
    const bounds = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockReturnValue({ top: 240 } as DOMRect);
    Object.defineProperty(window, "innerHeight", { configurable: true, value: 900 });
    try {
      const rendered = render(<GraphCanvas data={{ nodes: [], edges: [] }} centerIds={[]}
        relationshipTypeNames={{}} onNodeClick={vi.fn()} onEdgeClick={vi.fn()} />);
      expect(callbacks.height).toBe(636);
      expect((rendered.container.firstElementChild as HTMLElement).style.width).toBe("100%");
      expect((rendered.container.firstElementChild as HTMLElement).style.height).toBe("636px");
      Object.defineProperty(window, "innerHeight", { configurable: true, value: 1100 });
      fireEvent.resize(window);
      expect(callbacks.height).toBe(836);
    } finally {
      bounds.mockRestore();
      Object.defineProperty(window, "innerHeight", { configurable: true, value: previousHeight });
    }
  });
});
