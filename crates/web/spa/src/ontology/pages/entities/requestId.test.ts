// @vitest-environment jsdom
import "../../testSetup";
import { expect, it } from "vitest";
import { requestId } from "./requestId";
it("sets UUID version and variant without mutating the random input", () => {
  const bytes = new Uint8Array(16).fill(255);
  expect(requestId(bytes)).toBe("ffffffff-ffff-4fff-bfff-ffffffffffff");
  expect([...bytes]).toEqual(new Array(16).fill(255));
  expect(() => requestId(new Uint8Array(2))).toThrow();
});
