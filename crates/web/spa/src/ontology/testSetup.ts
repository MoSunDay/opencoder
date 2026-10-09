import "../test/setup-dom.js";
import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// Graph label layout measures glyphs; jsdom has no canvas renderer.
vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(() => ({
  measureText: (text: string) => ({ width: Array.from(text).length * 12 }),
}) as unknown as CanvasRenderingContext2D);
