// @vitest-environment jsdom
import "../../testSetup";
import { App, ConfigProvider } from "antd";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../api";
import type { Directory } from "../../types";
import DirectoryTree from "./DirectoryTree";

vi.mock("../../api", () => ({ api: {
  directoriesTree: vi.fn(), createDirectory: vi.fn(), updateDirectory: vi.fn(), moveDirectory: vi.fn(),
} }));

const directory = (id: string, name: string, parentId: string | null, extra: Partial<Directory> = {}): Directory =>
  ({ id, name, description: "", parent_id: parentId, revision: 1, is_deleted: false, ...extra });
const directories: Directory[] = [
  directory("root", "根目录", null),
  directory("dir-a", "平台", "root", { description: "平台目录", revision: 3 }),
  directory("dir-b", "服务", "root"),
  directory("dir-c", "子目录", "dir-a"),
];

beforeEach(() => {
  vi.mocked(api.createDirectory).mockResolvedValue({ item: { id: "new", name: "新模块", description: "", env_num: 1, entity_type_id: "type", revision: 1, is_deleted: false } });
  vi.mocked(api.updateDirectory).mockResolvedValue({ item: { id: "dir-a", name: "平台", description: "", env_num: 1, entity_type_id: "type", revision: 4, is_deleted: false } });
  vi.mocked(api.moveDirectory).mockResolvedValue({ relationship: { id: "rel", env_num: 1, relationship_type_id: "belongs", source_entity_id: "dir-c", target_entity_id: "dir-b", description: "", revision: 2, is_deleted: false, is_pinned: false } });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.resetAllMocks(); });

const mount = () => {
  const reload = vi.fn().mockResolvedValue(undefined);
  const onSelect = vi.fn();
  render(<ConfigProvider theme={{ token: { motion: false } }}><App>
    <DirectoryTree env="debug" canManage directories={directories} rootId="root" selectedId="root" onSelect={onSelect} reload={reload} />
  </App></ConfigProvider>);
  return { reload, onSelect };
};
const openMenu = async (label: string, item: string) => {
  fireEvent.click(screen.getByRole("button", { name: label }));
  fireEvent.click(await screen.findByText(item, { selector: ".ant-dropdown-menu-title-content" }));
};
const commit = (input: HTMLElement, value: string) => { fireEvent.change(input, { target: { value } }); fireEvent.keyDown(input, { key: "Enter", code: "Enter", keyCode: 13 }); };
const expand = (name: string) => {
  const row = screen.getByText(name).closest(".ant-tree-treenode") as HTMLElement;
  fireEvent.click(row.querySelector(".ant-tree-switcher") as HTMLElement);
};

it("creates a sub-directory inline with the parent id and reloads", async () => {
  const { reload } = mount();
  await openMenu("目录操作 平台", "新增子目录");
  const input = await screen.findByLabelText("新增目录名称");
  expect(input.closest(".dir-tree")).toBeTruthy();
  commit(input, "新模块");
  await waitFor(() => expect(api.createDirectory).toHaveBeenCalledWith("debug", { name: "新模块", parent_id: "dir-a" }));
  expect(reload).toHaveBeenCalled();
});

it("rejects duplicate sibling names before calling the API", async () => {
  mount();
  await openMenu("目录操作 平台", "新增子目录");
  const input = await screen.findByLabelText("新增目录名称");
  fireEvent.change(input, { target: { value: "子目录" } });
  expect(await screen.findByRole("alert")).toHaveTextContent("同名目录已存在");
  fireEvent.keyDown(input, { key: "Enter", code: "Enter", keyCode: 13 });
  expect(api.createDirectory).not.toHaveBeenCalled();
  expect(screen.getByLabelText("新增目录名称")).toBeInTheDocument();
});

it("renames through updateDirectory keeping description and revision", async () => {
  const { reload } = mount();
  await openMenu("目录操作 平台", "重命名");
  const input = await screen.findByLabelText("重命名目录名称");
  expect(input).toHaveValue("平台");
  commit(input, "平台能力");
  await waitFor(() => expect(api.updateDirectory).toHaveBeenCalledWith("debug", "dir-a", {
    name: "平台能力", description: "平台目录", is_deleted: false, expected_revision: 3,
  }));
  expect(reload).toHaveBeenCalled();
});

it("soft-deletes after confirmation and surfaces the revision", async () => {
  mount();
  await openMenu("目录操作 平台", "删除目录");
  const dialog = await screen.findByRole("dialog");
  fireEvent.click(within(dialog).getByRole("button", { name: /删\s*除/ }));
  await waitFor(() => expect(api.updateDirectory).toHaveBeenCalledWith("debug", "dir-a", {
    name: "平台", description: "平台目录", is_deleted: true, expected_revision: 3,
  }));
});

it("only allows creating children under the root node", async () => {
  mount();
  fireEvent.click(screen.getByRole("button", { name: "目录操作 根目录" }));
  expect(await screen.findByText("新增子目录", { selector: ".ant-dropdown-menu-title-content" })).toBeInTheDocument();
  expect(screen.queryByText("重命名", { selector: ".ant-dropdown-menu-title-content" })).not.toBeInTheDocument();
  expect(screen.queryByText("删除目录", { selector: ".ant-dropdown-menu-title-content" })).not.toBeInTheDocument();
  expect(screen.queryByText("移动到…", { selector: ".ant-dropdown-menu-title-content" })).not.toBeInTheDocument();
});

it("moves a directory via the move dialog to a selected candidate", async () => {
  const { reload } = mount();
  expand("平台");
  await openMenu("目录操作 子目录", "移动到…");
  const dialog = await screen.findByRole("dialog");
  fireEvent.mouseDown(within(dialog).getByRole("combobox", { name: "目标目录" }));
  const option = await waitFor(() => {
    const found = document.querySelector('.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option[title="服务"]');
    expect(found).not.toBeNull();
    return found as HTMLElement;
  });
  fireEvent.click(option);
  fireEvent.click(within(dialog).getByRole("button", { name: /移\s*动/ }));
  await waitFor(() => expect(api.moveDirectory).toHaveBeenCalledWith("debug", "dir-c", "dir-b"));
  expect(reload).toHaveBeenCalled();
});

it("selects a node on click", async () => {
  const { onSelect } = mount();
  fireEvent.click(await screen.findByText("服务"));
  await waitFor(() => expect(onSelect).toHaveBeenCalledWith("dir-b"));
});

it("starts an inline create under the root when the panel toolbar signals it", async () => {
  render(<ConfigProvider theme={{ token: { motion: false } }}><App>
    <DirectoryTree env="debug" canManage directories={directories} rootId="root" selectedId="root" onSelect={vi.fn()}
      reload={vi.fn().mockResolvedValue(undefined)} createRootSignal={1} />
  </App></ConfigProvider>);
  const input = await screen.findByLabelText("新增目录名称");
  commit(input, "顶层目录");
  await waitFor(() => expect(api.createDirectory).toHaveBeenCalledWith("debug", { name: "顶层目录", parent_id: "root" }));
});
