// @vitest-environment jsdom
import "../../testSetup";
import { App, ConfigProvider } from "antd";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../api";
import EntitiesPage from "../EntitiesPage";

vi.mock("../../api", () => ({ api: {
  entities: vi.fn(), entityTypes: vi.fn(), relationships: vi.fn(), relationshipTypes: vi.fn(), directoriesTree: vi.fn(),
} }));

beforeEach(() => {
  vi.mocked(api.entities).mockResolvedValue({ items: [{
    id: "unassigned", name: "未归入目录的实体", description: "", env_num: 1,
    entity_type_id: "type", revision: 1, is_deleted: false,
  }] });
  vi.mocked(api.entityTypes).mockResolvedValue({ items: [{ id: "type", env_num: 1, type_key: "type", name: "能力", description: "", is_system: false, revision: 1, is_deleted: false }] });
  vi.mocked(api.relationships).mockResolvedValue({ items: [{
    id: "rel-1", env_num: 1, relationship_type_id: "belongs", source_entity_id: "unassigned", target_entity_id: "dir-1",
    description: "", revision: 1, is_deleted: false, is_pinned: false,
  }] });
  vi.mocked(api.relationshipTypes).mockResolvedValue({ items: [{
    id: "belongs", env_num: 1, type_key: "belongs_to", name: "属于", description: "", is_directory_membership: true,
    is_system: true, revision: 1, is_deleted: false,
  }] });
  vi.mocked(api.directoriesTree).mockResolvedValue({ root_id: "root", items: [
    { id: "root", name: "根目录", description: "", parent_id: null, revision: 1, is_deleted: false },
    { id: "dir-1", name: "服务目录", description: "", parent_id: "root", revision: 1, is_deleted: false },
  ] });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.resetAllMocks(); });
const open = () => render(<ConfigProvider theme={{ token: { motion: false } }}><App><EntitiesPage /></App></ConfigProvider>);

it("shows all entities on first load and applies search", async () => {
  open();
  await waitFor(() => expect(api.entities).toHaveBeenCalled());
  expect(await screen.findByRole("button", { name: "未归入目录的实体" })).toBeInTheDocument();
  expect(screen.queryByRole("combobox", { name: "实体目录" })).not.toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "搜索名称或标识" }), { target: { value: "不存在" } });
  expect(screen.queryByRole("button", { name: "未归入目录的实体" })).not.toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "搜索名称或标识" }), { target: { value: "未归入" } });
  expect(await screen.findByRole("button", { name: "未归入目录的实体" })).toBeInTheDocument();
});

it("shows load errors and lets retry restore the list", async () => {
  vi.mocked(api.entities).mockRejectedValueOnce(new Error("实体服务暂不可用"));
  open();
  await screen.findByText("实体服务暂不可用");
  fireEvent.click(screen.getByRole("button", { name: /重\s*试/ }));
  expect(await screen.findByRole("button", { name: "未归入目录的实体" })).toBeInTheDocument();
  expect(screen.queryByText("实体加载失败")).not.toBeInTheDocument();
});

it("keeps the entities tab active by default and hides the directory workspace", async () => {
  open();
  await waitFor(() => expect(api.directoriesTree).toHaveBeenCalled());
  expect(screen.getByRole("tab", { name: "实体" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tab", { name: "目录" })).toHaveAttribute("aria-selected", "false");
  expect(screen.getByRole("button", { name: "未归入目录的实体" })).toBeInTheDocument();
  expect(screen.queryByText("根目录")).not.toBeInTheDocument();
});

it("shows directory tags and focuses the directory tree on click", async () => {
  open();
  fireEvent.click(await screen.findByRole("button", { name: "未归入目录的实体" }));
  const tag = screen.getByText("服务目录");
  expect(tag.closest(".entity-dir-tag")).toBeTruthy();
  fireEvent.click(tag);
  const panel = await screen.findByRole("tabpanel", { name: "目录" });
  expect(screen.getByRole("tab", { name: "目录" })).toHaveAttribute("aria-selected", "true");
  const tree = within(panel).getByRole("complementary", { name: "目录树" });
  expect(within(tree).getByText("根目录")).toBeInTheDocument();
  expect(within(tree).getByText("服务目录")).toBeInTheDocument();
  const members = within(panel).getByRole("region", { name: "目录成员" });
  expect(within(members).getByText("服务目录")).toBeInTheDocument();
  expect(within(members).getByText("1 个成员")).toBeInTheDocument();
});

it("renders the directory panel from the directory tab", async () => {
  open();
  await screen.findByRole("button", { name: "未归入目录的实体" });
  fireEvent.click(screen.getByRole("tab", { name: "目录" }));
  const panel = await screen.findByRole("tabpanel", { name: "目录" });
  const tree = within(panel).getByRole("complementary", { name: "目录树" });
  expect(within(tree).getByText("根目录")).toBeInTheDocument();
  expect(within(panel).getByRole("button", { name: /刷\s*新/ })).toBeInTheDocument();
  expect(within(panel).queryByRole("button", { name: "未归入目录的实体" })).not.toBeInTheDocument();
});
