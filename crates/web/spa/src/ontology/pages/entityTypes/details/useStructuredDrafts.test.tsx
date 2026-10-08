// @vitest-environment jsdom
import { ConfigProvider } from "antd";
import "../../../testSetup";
import { App } from "antd";
import { act, renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { api } from "../../../api";
import type { AttributeDefinition, Entity } from "../../../types";
import { useStructuredDrafts } from "./useStructuredDrafts";

vi.mock("../../../api", () => ({ api: { setAttribute: vi.fn() } }));
beforeEach(() => vi.resetAllMocks());
const wrapper = ({ children }: PropsWithChildren) => <ConfigProvider theme={{ token: { motion: false } }}><App>{children}</App></ConfigProvider>;
const entity: Entity = { id: "node", env_num: 1, entity_type_id: "type", name: "节点", description: "", revision: 1, is_deleted: false };
const owner: AttributeDefinition = { id: "owner", env_num: 1, entity_type_id: "type", attribute_key: "owner", name: "负责人", kind: "string",
  attribute_role: "custom", storage_mode: "sql", required: false, revision: 1, is_deleted: false, description: "" };

it("retains the draft's original revision on refresh and clears a reverted edit", async () => {
  vi.mocked(api.setAttribute).mockRejectedValue(new Error("版本冲突"));
  const { result, rerender } = renderHook(({ revision, value }) => useStructuredDrafts({ env: "debug", entity,
    definitions: [owner], rows: [{ attribute_definition_id: "owner", revision, value }], onSaved: async () => {},
  }), { initialProps: { revision: 1, value: "原负责人" }, wrapper });
  act(() => result.current.change(owner, "原负责人")); expect(result.current.dirty).toBe(false);
  act(() => result.current.change(owner, "新负责人"));
  rerender({ revision: 2, value: "其他人更新" });
  await act(async () => result.current.submit());
  expect(api.setAttribute).toHaveBeenCalledWith("debug", "node", "owner", { kind: "string", value: "新负责人", is_deleted: false, expected_revision: 1 });
  expect(result.current.valueOf("owner")).toBe("新负责人"); expect(result.current.dirty).toBe(true);
  act(() => result.current.change(owner, "原负责人"));
  expect(result.current.dirty).toBe(false); expect(result.current.valueOf("owner")).toBe("其他人更新");
});

it("stops the remaining batch and ignores its result after switching ENV", async () => {
  const second = { ...owner, id: "second", name: "备注" };
  let finish!: (value: object) => void;
  vi.mocked(api.setAttribute).mockReturnValue(new Promise((resolve) => { finish = resolve; }));
  const refresh = vi.fn();
  const { result, rerender } = renderHook(({ env }) => useStructuredDrafts({ env, entity,
    definitions: [owner, second], rows: [], onSaved: refresh,
  }), { initialProps: { env: "debug" }, wrapper });
  act(() => { result.current.change(owner, "负责人"); result.current.change(second, "备注"); });
  let saving!: Promise<void>;
  act(() => { saving = result.current.submit(); });
  rerender({ env: "other" });
  await act(async () => { finish({}); await saving; });
  expect(api.setAttribute).toHaveBeenCalledTimes(1); expect(refresh).not.toHaveBeenCalled();
  expect(result.current.dirty).toBe(false); expect(result.current.saving).toBe(false);
});
