// @vitest-environment jsdom
import "../../testSetup";
import { App, ConfigProvider, Grid } from "antd";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../api";
import EntitiesPage from "../EntitiesPage";

vi.mock("../../api", () => ({ api: {
  entities: vi.fn(), entityTypes: vi.fn(), directories: vi.fn(), relationships: vi.fn(), relationshipTypes: vi.fn(),
} }));

beforeEach(() => {
  vi.spyOn(Grid, "useBreakpoint").mockReturnValue({ md: false });
  vi.mocked(api.entities).mockResolvedValue({ items: [{
    id: "unassigned", name: "未归入目录的实体", description: "", env_num: 1,
    entity_type_id: "type", revision: 1, is_deleted: false,
  }] });
  vi.mocked(api.entityTypes).mockResolvedValue({ items: [] });
  vi.mocked(api.directories).mockResolvedValue({ items: [], root_id: "root" });
  vi.mocked(api.relationships).mockResolvedValue({ items: [] });
  vi.mocked(api.relationshipTypes).mockResolvedValue({ items: [] });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.resetAllMocks(); });
const open = () => render(<ConfigProvider theme={{ token: { motion: false } }}><App><EntitiesPage /></App></ConfigProvider>);

it("finds unassigned entities from a fresh narrow-screen page and applies search", async () => {
  open();
  await waitFor(() => expect(api.entities).toHaveBeenCalled());
  expect(screen.queryByRole("button", { name: "未归入目录的实体" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "全部实体" }));
  expect(await screen.findByRole("button", { name: "未归入目录的实体" })).toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "搜索名称或标识" }), { target: { value: "不存在" } });
  expect(screen.queryByRole("button", { name: "未归入目录的实体" })).not.toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "搜索名称或标识" }), { target: { value: "未归入" } });
  expect(await screen.findByRole("button", { name: "未归入目录的实体" })).toBeInTheDocument();
});

it("shows load errors and lets retry preserve the all-entities selection", async () => {
  vi.mocked(api.entities).mockRejectedValueOnce(new Error("目录服务暂不可用"));
  open();
  await screen.findByText("目录服务暂不可用");
  fireEvent.click(screen.getByRole("button", { name: "全部实体" }));
  fireEvent.click(screen.getByRole("button", { name: /重\s*试/ }));
  expect(await screen.findByRole("button", { name: "未归入目录的实体" })).toBeInTheDocument();
  expect(screen.queryByText("实体加载失败")).not.toBeInTheDocument();
});
