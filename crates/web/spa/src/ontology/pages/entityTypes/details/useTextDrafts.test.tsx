// @vitest-environment jsdom
import { ConfigProvider } from "antd";
import "../../../testSetup";
import { App } from "antd";
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import { api } from "../../../api";
import type { Entity } from "../../../types";
import type { TextAttribute } from "./types";
import { useTextDrafts } from "./useTextDrafts";

vi.mock("../../../api", () => ({ api: { textContent: vi.fn(), setText: vi.fn(), setNfsPath: vi.fn() } }));
beforeEach(() => vi.resetAllMocks());
const wrapper = ({ children }: PropsWithChildren) => <ConfigProvider theme={{ token: { motion: false } }}><App>{children}</App></ConfigProvider>;
const entity: Entity = { id: "node", env_num: 1, entity_type_id: "type", name: "节点", description: "", revision: 1, is_deleted: false };
const source: TextAttribute["definition"] = { id: "source", env_num: 1, entity_type_id: "type", attribute_key: "source", name: "来源", kind: "text",
  attribute_role: "source", storage_mode: "markdown", required: true, revision: 1, is_deleted: false, description: "" };
const item = (revision: number): TextAttribute => ({ definition: source, current: { revision, bytes: 8 } });

it("cancels to the latest known revision and uses it for the next save", async () => {
  vi.mocked(api.textContent).mockResolvedValueOnce({ format: "md", content: "版本一正文", revision: 1 })
    .mockResolvedValue({ format: "md", content: "版本二正文", revision: 2 });
  vi.mocked(api.setText).mockResolvedValue({ revision: 3 });
  const { result, rerender } = renderHook(({ revision }) => useTextDrafts({
    env: "jy-hub", entity, items: [item(revision)], reload: async () => {}, onChanged: async () => {},
  }), { initialProps: { revision: 1 }, wrapper });
  await waitFor(() => expect(result.current.values.source).toBe("版本一正文"));
  act(() => { result.current.edit("source"); result.current.change("source", "待保存草稿"); });
  rerender({ revision: 2 });
  expect(result.current.values.source).toBe("待保存草稿"); expect(api.textContent).toHaveBeenCalledTimes(1);
  act(() => result.current.cancel("source"));
  await waitFor(() => expect(result.current.values.source).toBe("版本二正文"));
  act(() => { result.current.edit("source"); result.current.change("source", "合并后的正文"); });
  await act(async () => result.current.save(item(2)));
  expect(api.setText).toHaveBeenCalledWith("jy-hub", "node", "source", { format: "md", content: "合并后的正文", expected_revision: 2 });
  expect(result.current.values.source).toBe("合并后的正文");
  expect(api.textContent).toHaveBeenCalledTimes(2);
});

it("retains a conflicting draft while metadata refreshes and loads the new body on cancel", async () => {
  vi.mocked(api.textContent).mockImplementation(async (_env, _entity, _id, revision) => ({ format: "md", content: `正文 ${revision}`, revision }));
  vi.mocked(api.setText).mockRejectedValue(new Error("版本冲突"));
  const reload = vi.fn();
  const { result, rerender } = renderHook(({ revision }) => useTextDrafts({
    env: "jy-hub", entity, items: [item(revision)], reload, onChanged: async () => {},
  }), { initialProps: { revision: 1 }, wrapper });
  await waitFor(() => expect(result.current.values.source).toBe("正文 1"));
  act(() => { result.current.edit("source"); result.current.change("source", "保留的草稿"); });
  await act(async () => result.current.save(item(1)));
  expect(result.current.errors.source).toBe("版本冲突"); expect(result.current.dirty).toBe(true);
  await act(async () => result.current.refresh()); expect(reload).toHaveBeenCalledTimes(1);
  rerender({ revision: 2 }); expect(result.current.values.source).toBe("保留的草稿");
  act(() => result.current.cancel("source"));
  await waitFor(() => expect(result.current.values.source).toBe("正文 2"));
});

it("keeps the committed text and revision when refreshing metadata fails", async () => {
  vi.mocked(api.textContent).mockImplementation(async (_env, _entity, _id, revision) => ({
    format: "md", content: revision === 1 ? "旧正文" : "已提交正文", revision,
  }));
  vi.mocked(api.setText).mockResolvedValue({ revision: 2 });
  const { result } = renderHook(() => useTextDrafts({ env: "jy-hub", entity, items: [item(1)],
    reload: async () => { throw new Error("网络断开"); }, onChanged: async () => {},
  }), { wrapper });
  await waitFor(() => expect(result.current.values.source).toBe("旧正文"));
  act(() => { result.current.edit("source"); result.current.change("source", "已提交正文"); });
  await act(async () => result.current.save(item(1)));
  expect(result.current.values.source).toBe("已提交正文"); expect(result.current.editing.source).toBe(false);
  expect(result.current.errors.source).toContain("内容已保存，详情刷新失败");
  expect(api.textContent).toHaveBeenCalledTimes(1);
  act(() => result.current.retry(item(1)));
  await waitFor(() => expect(api.textContent).toHaveBeenLastCalledWith("jy-hub", "node", "source", 2));
  await waitFor(() => expect(result.current.loading.source).toBe(false));
  expect(result.current.values.source).toBe("已提交正文");
  act(() => { result.current.edit("source"); result.current.change("source", "再次编辑"); });
  await act(async () => result.current.save(item(1)));
  expect(vi.mocked(api.setText).mock.calls[1][3]).toMatchObject({ expected_revision: 2 });
});

it("reads the newly bound NFS version and preserves the relative path", async () => {
  const nfs: TextAttribute = { definition: { ...source, id: "ext", attribute_key: "ext", attribute_role: "ext", storage_mode: "nfs_path" },
    current: { revision: 1, content_path: "old.txt" } };
  vi.mocked(api.textContent).mockResolvedValueOnce({ format: "nfs_path", content: "旧文件正文", revision: 1 })
    .mockResolvedValue({ format: "nfs_path", content: "新文件正文", revision: 2 });
  vi.mocked(api.setNfsPath).mockResolvedValue({ revision: 2 });
  const { result } = renderHook(() => useTextDrafts({ env: "jy-hub", entity, items: [nfs],
    reload: async () => {}, onChanged: async () => {},
  }), { wrapper });
  await waitFor(() => expect(result.current.values.ext).toBe("old.txt"));
  act(() => { result.current.edit("ext"); result.current.change("ext", "new.txt"); });
  await act(async () => result.current.save(nfs));
  expect(api.setNfsPath).toHaveBeenCalledWith("jy-hub", "node", "ext", { path: "new.txt", expected_revision: 1 });
  expect(result.current.values.ext).toBe("new.txt"); expect(result.current.bodies.ext).toBe("新文件正文");
  expect(api.setText).not.toHaveBeenCalled();
  act(() => result.current.retry(nfs));
  await waitFor(() => expect(result.current.loading.ext).toBe(false));
  expect(api.textContent).toHaveBeenLastCalledWith("jy-hub", "node", "ext", 2);
  expect(result.current.values.ext).toBe("new.txt");
});

it("ignores a write completion from a previous selection even when the same entity is reselected", async () => {
  vi.mocked(api.textContent).mockResolvedValue({ format: "md", content: "当前正文", revision: 1 });
  let finish!: (value: { revision: number }) => void;
  vi.mocked(api.setText).mockReturnValue(new Promise((resolve) => { finish = resolve; }));
  const changed = vi.fn();
  const { result, rerender } = renderHook(({ selected }) => useTextDrafts({
    env: "jy-hub", entity: selected, items: [item(1)], reload: async () => {}, onChanged: changed,
  }), { initialProps: { selected: entity }, wrapper });
  await waitFor(() => expect(result.current.values.source).toBe("当前正文"));
  act(() => { result.current.edit("source"); result.current.change("source", "旧选择的草稿"); });
  let saving!: Promise<void>;
  act(() => { saving = result.current.save(item(1)); });
  rerender({ selected: { ...entity, id: "other" } }); rerender({ selected: entity });
  await waitFor(() => expect(result.current.values.source).toBe("当前正文"));
  act(() => { result.current.edit("source"); result.current.change("source", "新选择的草稿"); });
  await act(async () => { finish({ revision: 2 }); await saving; });
  expect(result.current.values.source).toBe("新选择的草稿"); expect(changed).not.toHaveBeenCalled();
});
