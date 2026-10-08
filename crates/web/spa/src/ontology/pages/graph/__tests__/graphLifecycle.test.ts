// @vitest-environment jsdom
import "../../../testSetup";
import { describe, expect, it, vi } from "vitest";
import { coordinateGraphLifecycle } from "../graphLifecycle";

function fixture() {
  const graph = { destroyed: false, render: vi.fn().mockResolvedValue(undefined), destroy: vi.fn(() => { graph.destroyed = true; }) };
  const nativeRender = graph.render;
  const nativeDestroy = graph.destroy;
  coordinateGraphLifecycle(graph);
  return { graph, nativeRender, nativeDestroy };
}

describe("async graph lifecycle", () => {
  it("cancels a queued render when StrictMode destroys the instance before it starts", async () => {
    const { graph, nativeRender, nativeDestroy } = fixture();
    const ready = graph.render();
    graph.destroy();
    await ready;
    expect(nativeRender).not.toHaveBeenCalled();
    expect(nativeDestroy).toHaveBeenCalledTimes(1);
  });

  it("waits for an active render before destroying and cancels the remaining queue", async () => {
    const { graph, nativeRender, nativeDestroy } = fixture();
    let finish!: () => void;
    nativeRender.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
    const first = graph.render();
    await Promise.resolve();
    const second = graph.render();
    graph.destroy();
    expect(nativeDestroy).not.toHaveBeenCalled();
    finish();
    await Promise.all([first, second]);
    expect(nativeRender).toHaveBeenCalledTimes(1);
    expect(nativeDestroy).toHaveBeenCalledTimes(1);
    expect(graph.destroyed).toBe(true);
  });

  it("preserves rendering failures and still releases a disposing graph", async () => {
    const { graph, nativeRender, nativeDestroy } = fixture();
    let fail!: (error: Error) => void;
    nativeRender.mockImplementationOnce(() => new Promise<void>((_, reject) => { fail = reject; }));
    const rendering = graph.render();
    const rejected = expect(rendering).rejects.toThrow("render failed");
    await Promise.resolve();
    graph.destroy();
    fail(new Error("render failed"));
    await rejected;
    expect(nativeDestroy).toHaveBeenCalledTimes(1);
  });
});
