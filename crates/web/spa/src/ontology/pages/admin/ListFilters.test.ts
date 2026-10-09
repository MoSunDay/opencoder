// @vitest-environment jsdom
import "../../testSetup";
import { expect, it } from "vitest";
import { matchesRecord } from "./ListFilters";
it("applies status and case-insensitive text together", () => {
  expect(matchesRecord({ is_deleted: false }, "active", "RPC", ["Core rpc", "jy_rpc"])).toBe(true);
  expect(matchesRecord({ is_deleted: true }, "active", "", ["RPC"])).toBe(false);
  expect(matchesRecord({ is_deleted: false }, "active", "missing", ["RPC"])).toBe(false);
  expect(matchesRecord({ is_deleted: true }, "all", "jy_rpc", ["Core RPC", "jy_rpc"])).toBe(true);
});
