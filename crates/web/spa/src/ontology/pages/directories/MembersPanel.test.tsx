// @vitest-environment jsdom
import "../../testSetup";
import { App, ConfigProvider } from "antd";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../api";
import type { Directory, Entity, EntityType, Relationship } from "../../types";
import MembersPanel from "./MembersPanel";

vi.mock("../../api", () => ({ api: { createRelationship: vi.fn(), updateRelationship: vi.fn() } }));

const entity = (id: string, name: string, extra: Partial<Entity> = {}): Entity =>
  ({ id, name, description: "", env_num: 1, entity_type_id: "type-a", revision: 1, is_deleted: false, ...extra });
const directory: Directory = { id: "dir-a", name: "平台", description: "", parent_id: "root", revision: 1, is_deleted: false };
const entities = [entity("e1", "成员实体"), entity("e2", "候选实体"), entity("e3", "已删除实体", { is_deleted: true })];
const entityTypes: EntityType[] = [{ id: "type-a", env_num: 1, type_key: "type-a", name: "能力", description: "", is_system: false, revision: 1, is_deleted: false }];
const memberships: Relationship[] = [{
  id: "rel-1", env_num: 1, relationship_type_id: "belongs", source_entity_id: "e1", target_entity_id: "dir-a",
  description: "", revision: 2, is_deleted: false, is_pinned: false,
}];

beforeEach(() => {
  vi.mocked(api.createRelationship).mockResolvedValue({ item: { ...memberships[0], id: "rel-2", source_entity_id: "e2" } });
  vi.mocked(api.updateRelationship).mockResolvedValue({ item: { ...memberships[0], is_deleted: true } });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.resetAllMocks(); });

const mount = (props: Record<string, unknown> = {}) => {
  const reload = vi.fn().mockResolvedValue(undefined);
  render(<ConfigProvider theme={{ token: { motion: false } }}><App>
    <MembersPanel env="debug" canManage directory={directory} entities={entities} entityTypes={entityTypes}
      memberships={memberships} membershipTypeId="belongs" reload={reload} {...props} />
  </App></ConfigProvider>);
  return reload;
};
const choose = async (name: string) => {
  const option = await waitFor(() => {
    const found = document.querySelector(`.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option[title="${name}"]`);
    expect(found).not.toBeNull();
    return found as HTMLElement;
  });
  fireEvent.click(option);
};

it("lists current members with their entity type", () => {
  mount();
  expect(screen.getByText("平台")).toBeInTheDocument();
  expect(screen.getByText("成员实体")).toBeInTheDocument();
  expect(screen.getByText("能力")).toBeInTheDocument();
  expect(screen.getByText("1 个成员")).toBeInTheDocument();
});

it("titles the panel as 根目录 for the root directory and shows an empty state", () => {
  mount({ directory: { ...directory, id: "root", name: "根目录" }, isRoot: true, memberships: [] });
  expect(screen.getByText("根目录")).toBeInTheDocument();
  expect(screen.getByText("目录暂无成员")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /添\s*加/ })).toBeDisabled();
});

it("adds entities that are not members yet and reloads", async () => {
  const reload = mount();
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "添加实体" }));
  await choose("候选实体");
  expect(screen.queryByText("成员实体")).toBeTruthy();
  const dropdown = document.querySelector(".ant-select-dropdown:not(.ant-select-dropdown-hidden)");
  expect(dropdown?.textContent).not.toContain("成员实体");
  expect(dropdown?.textContent).not.toContain("已删除实体");
  fireEvent.click(screen.getByRole("button", { name: /添\s*加/ }));
  await waitFor(() => expect(api.createRelationship).toHaveBeenCalledWith("debug", {
    relationship_type_id: "belongs", source_entity_id: "e2", target_entity_id: "dir-a",
  }));
  expect(reload).toHaveBeenCalled();
});

it("removes a member by soft-deleting the membership relationship", async () => {
  const reload = mount();
  fireEvent.click(screen.getByRole("button", { name: "移出 成员实体" }));
  await waitFor(() => expect(api.updateRelationship).toHaveBeenCalledWith("debug", "rel-1", { is_deleted: true, expected_revision: 2 }));
  expect(reload).toHaveBeenCalled();
});
