// @vitest-environment jsdom
import { ConfigProvider } from "antd";
import "../../testSetup";
import { App as AntdApp } from "antd";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../../api";
import { EnvContext } from "../../env";
import CreateRelationshipModal from "./CreateRelationshipModal";
import RelationshipTypesPage from "../RelationshipTypesPage";

vi.mock("../../api", () => ({
  api: {
    relationshipTypes: vi.fn(),
    entityTypes: vi.fn(),
    entities: vi.fn(),
    createRelationship: vi.fn(),
    createRelationshipType: vi.fn(),
    updateRelationshipType: vi.fn(),
  },
}));

const entityTypes = [
  { id: "t1", env_num: 1, type_key: "source_business", name: "类型一", description: "", is_system: false, revision: 1, is_deleted: false },
  { id: "t2", env_num: 1, type_key: "target_service", name: "类型二", description: "", is_system: false, revision: 1, is_deleted: false },
  { id: "t3", env_num: 1, type_key: "t3", name: "类型三", description: "", is_system: false, revision: 1, is_deleted: false },
];
const entities = [
  { id: "e1", env_num: 1, entity_type_id: "t1", name: "实体一", description: "", revision: 1, is_deleted: false },
  { id: "e2", env_num: 1, entity_type_id: "t2", name: "实体二", description: "", revision: 1, is_deleted: false },
  { id: "e3", env_num: 1, entity_type_id: "t3", name: "实体三", description: "", revision: 1, is_deleted: false },
];
const relationshipType = {
  id: "rt-1", env_num: 1, type_key: "deploy_to", name: "部署依赖", description: "", is_directory_membership: false, is_system: false,
  source_entity_type_id: "t1", target_entity_type_ids: ["t2"], revision: 1, is_deleted: false,
};

function openModal(overrides: Partial<typeof relationshipType> = {}) {
  render(
    <ConfigProvider theme={{ token: { motion: false } }}><AntdApp>
      <CreateRelationshipModal
        env="debug"
        relationshipType={{ ...relationshipType, ...overrides }}
        entityTypes={entityTypes}
        entities={entities}
        onCreated={vi.fn().mockResolvedValue(undefined)}
      />
    </AntdApp></ConfigProvider>,
  );
}

async function openDialog() {
  fireEvent.click(await screen.findByRole("button", { name: "创建关系" }));
  return screen.findByRole("dialog");
}

async function openOptions(combo: HTMLElement) {
  fireEvent.mouseDown(combo);
  const dropdown = await waitFor(() => {
    const id = combo.getAttribute("aria-controls");
    const el = id ? document.getElementById(id)?.closest<HTMLElement>(".ant-select-dropdown") : null;
    expect(el).not.toBeNull();
    return el!;
  });
  return within(dropdown);
}

describe("CreateRelationshipModal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.createRelationship).mockResolvedValue({
      item: {
        id: "relationship", env_num: 1, relationship_type_id: "rt-1", source_entity_id: "e1", target_entity_id: "e2",
        description: "", revision: 1, is_deleted: false, is_pinned: false,
      },
    });
  });

  it("filters both endpoints by the declared type constraints", async () => {
    openModal();
    const dialog = await openDialog();
    expect(await within(dialog).findByText("端点约束：类型一 → 类型二")).toBeInTheDocument();
    const combos = within(dialog).getAllByRole("combobox");
    fireEvent.mouseDown(combos[0]!);
    expect(await screen.findByText("实体一")).toBeInTheDocument();
    expect(screen.queryByText("实体二")).not.toBeInTheDocument();
    expect(screen.queryByText("实体三")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("实体一"));
    fireEvent.mouseDown(combos[1]!);
    expect(await screen.findByText("实体二")).toBeInTheDocument();
    expect(screen.queryByText("实体三")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("实体二"));
    fireEvent.change(within(dialog).getByRole("textbox"), { target: { value: "新的依赖" } });
    fireEvent.click(dialog.querySelector<HTMLButtonElement>("button.ant-btn-primary")!);
    await waitFor(() => expect(api.createRelationship).toHaveBeenCalledWith("debug", {
      relationship_type_id: "rt-1",
      source_entity_id: "e1",
      target_entity_id: "e2",
      description: "新的依赖",
    }));
    expect(await screen.findByText("关系已创建")).toBeInTheDocument();
  });

  it("lists every entity for a type without declared constraints", async () => {
    openModal({ source_entity_type_id: undefined, target_entity_type_ids: undefined });
    const dialog = await openDialog();
    expect(await within(dialog).findByText("该类型未声明端点约束，提交后以后端校验为准")).toBeInTheDocument();
    const combos = within(dialog).getAllByRole("combobox");
    const sourceScope = await openOptions(combos[0]!);
    expect(await sourceScope.findByText("实体一")).toBeInTheDocument();
    expect(sourceScope.getByText("实体二")).toBeInTheDocument();
    expect(sourceScope.getByText("实体三")).toBeInTheDocument();
    fireEvent.click(sourceScope.getByText("实体一"));
    await waitFor(() => expect(combos[0]).toHaveAttribute("aria-expanded", "false"));
    const targetScope = await openOptions(combos[1]!);
    expect(await targetScope.findByText("实体二")).toBeInTheDocument();
    expect(targetScope.getByText("实体三")).toBeInTheDocument();
  });
});

describe("RelationshipTypesPage", () => {
  const normalType = {
    id: "rt-normal", env_num: 1, type_key: "deploy_to", name: "部署依赖", description: "", is_directory_membership: false,
    is_system: false, source_entity_type_id: "t1", target_entity_type_ids: ["t2"], revision: 1, is_deleted: false,
  };
  const membershipType = {
    id: "rt-dir", env_num: 1, type_key: "directory_membership", name: "归属关系", description: "", is_directory_membership: true,
    is_system: true, revision: 1, is_deleted: false,
  };

  it("hides the create-relationship entry for directory membership types", async () => {
    vi.mocked(api.relationshipTypes).mockResolvedValue({ items: [normalType, membershipType] });
    vi.mocked(api.entityTypes).mockResolvedValue({ items: entityTypes });
    vi.mocked(api.entities).mockResolvedValue({ items: entities });
    render(
      <ConfigProvider theme={{ token: { motion: false } }}><AntdApp>
        <EnvContext.Provider value={{ env: "debug", environments: [], setEnv: vi.fn(), refreshEnvironments: vi.fn(), canManage: true }}>
          <RelationshipTypesPage />
        </EnvContext.Provider>
      </AntdApp></ConfigProvider>,
    );
    expect(await screen.findByText("部署依赖")).toBeInTheDocument();
    expect(screen.getByText("归属关系")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "创建关系" })).toHaveLength(1);
  });
  it("searches known types by key and submits their IDs with target scope", async () => {
    vi.mocked(api.relationshipTypes).mockResolvedValue({ items: [] });
    vi.mocked(api.entityTypes).mockResolvedValue({ items: [...entityTypes, { ...entityTypes[0]!, id: "gone", name: "已停用候选", is_deleted: true }] });
    vi.mocked(api.entities).mockResolvedValue({ items: entities });
    vi.mocked(api.createRelationshipType).mockResolvedValue({ item: normalType });
    render(<ConfigProvider theme={{ token: { motion: false } }}><AntdApp><EnvContext.Provider value={{ env: "debug", environments: [], setEnv: vi.fn(), refreshEnvironments: vi.fn(), canManage: true }}><RelationshipTypesPage /></EnvContext.Provider></AntdApp></ConfigProvider>);
    fireEvent.click(await screen.findByRole("button", { name: /新增/ }));
    const dialog = await screen.findByRole("dialog");
    const fields = within(dialog).getAllByRole("textbox");
    fireEvent.change(fields[0]!, { target: { value: "deploy_to" } });
    fireEvent.change(fields[1]!, { target: { value: "部署依赖" } });
    const combos = within(dialog).getAllByRole("combobox");
    await openOptions(combos[0]!);
    fireEvent.change(combos[0]!, { target: { value: "source_business" } });
    const source = await waitFor(() => {
      const dropdown = document.body.querySelector<HTMLElement>(".ant-select-dropdown:not(.ant-select-dropdown-hidden)")!;
      expect(within(dropdown).queryByText("类型二")).not.toBeInTheDocument();
      expect(within(dropdown).queryByText("已停用候选")).not.toBeInTheDocument();
      return within(dropdown).getByText("类型一");
    });
    fireEvent.click(source);
    const targets = await openOptions(combos[1]!);
    fireEvent.change(combos[1]!, { target: { value: "target_service" } });
    fireEvent.click(await targets.findByText("类型二"));
    fireEvent.click(dialog.querySelector<HTMLButtonElement>("button.ant-btn-primary")!);
    await waitFor(() => expect(api.createRelationshipType).toHaveBeenCalledWith("debug", expect.objectContaining({
      key: "deploy_to", source_entity_type_id: "t1", target_entity_type_ids: ["t2"],
    })));
  });

  it("preserves the immutable global scope when editing a system template", async () => {
    vi.mocked(api.relationshipTypes).mockResolvedValue({ items: [membershipType] });
    vi.mocked(api.entityTypes).mockResolvedValue({ items: entityTypes });
    vi.mocked(api.entities).mockResolvedValue({ items: entities });
    render(<ConfigProvider theme={{ token: { motion: false } }}><AntdApp><EnvContext.Provider value={{ env: "debug", environments: [], setEnv: vi.fn(), refreshEnvironments: vi.fn(), canManage: true }}><RelationshipTypesPage /></EnvContext.Provider></AntdApp></ConfigProvider>);
    fireEvent.click(await screen.findByRole("button", { name: "详情" }));
    fireEvent.click(await screen.findByRole("button", { name: "编辑" }));
    const dialog = (await screen.findByText("编辑关系类型", { selector: ".ant-modal-title" })).closest<HTMLElement>(".ant-modal")!;
    expect(within(dialog).queryByRole("combobox")).not.toBeInTheDocument();
    fireEvent.change(within(dialog).getAllByRole("textbox")[0]!, { target: { value: "目录归属说明" } });
    fireEvent.click(dialog.querySelector<HTMLButtonElement>("button.ant-btn-primary")!);
    await waitFor(() => expect(api.updateRelationshipType).toHaveBeenCalledWith("debug", "rt-dir", expect.objectContaining({
      name: "目录归属说明", source_entity_type_id: null, target_entity_type_ids: [], expected_revision: 1, is_deleted: false,
    })));
  });

  it("retains the endpoint scope and revision when disabling a template", async () => {
    vi.mocked(api.relationshipTypes).mockResolvedValue({ items: [normalType] });
    vi.mocked(api.entityTypes).mockResolvedValue({ items: entityTypes });
    vi.mocked(api.entities).mockResolvedValue({ items: entities });
    render(<ConfigProvider theme={{ token: { motion: false } }}><AntdApp><EnvContext.Provider value={{ env: "debug", environments: [], setEnv: vi.fn(), refreshEnvironments: vi.fn(), canManage: true }}><RelationshipTypesPage /></EnvContext.Provider></AntdApp></ConfigProvider>);
    fireEvent.mouseEnter(await screen.findByRole("button", { name: "更多操作" }));
    fireEvent.click(await screen.findByText("停用类型"));
    const confirmation = await screen.findByText("已有关系保留，仅停用此类型。");
    const popup = confirmation.closest<HTMLElement>(".ant-modal")!;
    fireEvent.click(popup.querySelector<HTMLButtonElement>("button.ant-btn-primary")!);
    await waitFor(() => expect(api.updateRelationshipType).toHaveBeenCalledWith("debug", "rt-normal", {
      name: "部署依赖", description: "", source_entity_type_id: "t1", target_entity_type_ids: ["t2"], expected_revision: 1, is_deleted: true,
    }));
  });

});
