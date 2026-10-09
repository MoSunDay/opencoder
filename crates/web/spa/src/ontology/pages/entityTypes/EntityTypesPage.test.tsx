// @vitest-environment jsdom
import { ConfigProvider } from "antd";
import "../../testSetup";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App as AntApp } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../../api";
import { EnvContext } from "../../env";
import EntityTypesPage from "../EntityTypesPage";

// These cases open nested drawers with several retained forms under jsdom.
vi.setConfig({ testTimeout: 15000 });

vi.mock("../../api", () => ({
  api: {
    entityTypes: vi.fn(),
    attributes: vi.fn(),
    entities: vi.fn(),
    entity: vi.fn(),
    setAttribute: vi.fn(),
    setText: vi.fn(),
    createEntityType: vi.fn(),
    updateEntityType: vi.fn(),
    createAttribute: vi.fn(),
    updateAttribute: vi.fn(),
    relationships: vi.fn(),
    relationshipTypes: vi.fn(),
    actions: vi.fn(),
  },
}));

const systemType = {
  id: "00000000-0000-4000-8000-000000000001",
  env_num: 1,
  type_key: "platform_service",
  name: "关联系统",
  description: "工作流读取、写入或托管所依赖的系统",
  is_system: true,
  revision: 1,
  is_deleted: false,
};

const entity = {
  id: "00000000-0000-4000-8000-000000000010",
  env_num: 1,
  entity_type_id: systemType.id,
  name: "opencoder-cli",
  description: "执行调度服务",
  revision: 3,
  is_deleted: false,
};

const roleAttribute = {
  id: "1",
  env_num: 1,
  entity_type_id: systemType.id,
  attribute_key: "role",
  name: "系统角色",
  description: "",
  kind: "string" as const,
  required: true,
  revision: 1,
  is_deleted: false,
  attribute_role: "custom" as const,
  storage_mode: "sql" as const,
};

const relationshipType = {
  id: "00000000-0000-4000-8000-000000000030",
  env_num: 1,
  type_key: "depends_on",
  name: "依赖关系",
  description: "",
  is_directory_membership: false,
  is_system: true,
  revision: 1,
  is_deleted: false,
  target_entity_type_ids: [],
};

const relationship = {
  id: "00000000-0000-4000-8000-000000000020",
  env_num: 1,
  relationship_type_id: relationshipType.id,
  source_entity_id: entity.id,
  target_entity_id: "00000000-0000-4000-8000-000000000099",
  description: "opencoder-cli 依赖外部系统",
  revision: 1,
  is_deleted: false,
  is_pinned: false,
};

const renderPage = () =>
  render(
    <ConfigProvider theme={{ token: { motion: false } }}><AntApp>
      <EnvContext.Provider
        value={{
          env: "debug",
          environments: [],
          canManage: true,
          setEnv: () => undefined,
          refreshEnvironments: async () => undefined,
        }}
      >
        <EntityTypesPage />
      </EnvContext.Provider>
    </AntApp></ConfigProvider>,
  );

const typeDrawer = () => within(document.querySelector<HTMLElement>(".ant-drawer-open")!);
const entityDrawer = () => within([...document.querySelectorAll<HTMLElement>(".ant-drawer-open")].at(-1)!);

const baseMocks = () => {
  vi.mocked(api.entityTypes).mockResolvedValue({ items: [systemType] });
  vi.mocked(api.entities).mockResolvedValue({ items: [entity] });
  vi.mocked(api.attributes).mockResolvedValue({ items: [roleAttribute] });
  vi.mocked(api.relationships).mockResolvedValue({ items: [relationship] });
  vi.mocked(api.relationshipTypes).mockResolvedValue({ items: [relationshipType] });
  vi.mocked(api.actions).mockResolvedValue({ items: [{ id: "1", env_num: 1, entity_type_id: systemType.id, operation_type: "read", operation: "扩容", description: "", revision: 1, is_deleted: false }, { id: "2", env_num: 1, entity_type_id: systemType.id, operation_type: "write", operation: "发布", description: "", revision: 1, is_deleted: false }] });
};

const openTypeDetail = async () => {
  renderPage();
  fireEvent.click(await screen.findByRole("button", { name: /详情/ }));
};

const openEntityDetail = async () => {
  await openTypeDetail();
  fireEvent.click(typeDrawer().getByRole("tab", { name: /实体/ }));
  fireEvent.click(await typeDrawer().findByRole("button", { name: /详情/ }));
  await waitFor(() => expect(document.querySelectorAll(".ant-drawer-open")).toHaveLength(2));
};

describe("EntityTypesPage 实体类型详情抽屉", () => {
  it("主表保留详情，停用收进更多操作", async () => {
    const customType = { ...systemType, id: "00000000-0000-4000-8000-000000000009", type_key: "custom_type", name: "自定义类型", is_system: false };
    vi.mocked(api.entityTypes).mockResolvedValue({ items: [customType] });
    vi.mocked(api.entities).mockResolvedValue({ items: [] });
    vi.mocked(api.attributes).mockResolvedValue({ items: [] });
    vi.mocked(api.relationships).mockResolvedValue({ items: [] });
    vi.mocked(api.relationshipTypes).mockResolvedValue({ items: [] });
    renderPage();
    await screen.findByText("自定义类型");
    expect(screen.getByRole("button", { name: /详情/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "更多操作" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /实体/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^属性$/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /编辑/ })).not.toBeInTheDocument();
  });

  it("点类型详情打开右侧抽屉，实体Tab展示实体列表及来源与 Action", async () => {
    baseMocks();
    await openTypeDetail();
    const drawer = typeDrawer();
    expect(await drawer.findByText("opencoder-cli")).toBeInTheDocument();
    expect(drawer.queryByText(/source-contract/)).not.toBeInTheDocument();
    expect(api.entities).toHaveBeenCalledWith("debug", true);
  });

  it("实体详情按四个页签展示内容，关联系统展示来源与 Action", async () => {
    baseMocks();
    vi.mocked(api.entity).mockResolvedValue({
      item: entity,
      structured_attributes: [{ attribute_definition_id: "1", value: "调度", revision: 2 }],
    });
    await openEntityDetail();
    await entityDrawer().findByText("opencoder-cli");
    expect(await entityDrawer().findByRole("tab", { name: "基本信息" })).toHaveAttribute("aria-selected", "true");
    const drawerExtra = [...document.querySelectorAll<HTMLElement>(".ant-drawer-extra")].at(-1);
    expect(within(drawerExtra!).getByRole("button", { name: /编辑/ })).toBeInTheDocument();
    fireEvent.click(entityDrawer().getByRole("tab", { name: "来源" }));
    expect(entityDrawer().getAllByText(/待完善/).length).toBeGreaterThan(0);
    const wrappers = document.querySelectorAll(".ant-drawer-content-wrapper");
    expect(wrappers.length).toBeGreaterThanOrEqual(2);
    expect(wrappers[0]).toHaveStyle({ width: "100%" });
    expect(wrappers[wrappers.length - 1]).toHaveStyle({ width: "100%" });
  });

  it("关联系统属性经 List 内联编辑后提交调用 setAttribute", async () => {
    baseMocks();
    vi.mocked(api.entity).mockResolvedValue({
      item: entity,
      structured_attributes: [{ attribute_definition_id: "1", value: "调度", revision: 2 }],
    });
    await openEntityDetail();
    fireEvent.click(await entityDrawer().findByRole("tab", { name: "普通属性" }));
    const input = await entityDrawer().findByDisplayValue("调度");
    fireEvent.change(input, { target: { value: "托管" } });
    fireEvent.click(entityDrawer().getByRole("button", { name: /提交/ }));
    await waitFor(() =>
      expect(api.setAttribute).toHaveBeenCalledWith("debug", entity.id, "1", {
        kind: "string",
        value: "托管",
        is_deleted: false,
        expected_revision: 2,
      }),
    );
  });

  it("自定义类型的普通属性支持内联编辑", async () => {
    const workflowType = {
      ...systemType,
      id: "00000000-0000-4000-8000-000000000002",
      type_key: "platform_workflow",
      name: "测试工作流",
    };
    const workflowEntity = { ...entity, id: "00000000-0000-4000-8000-000000000011", entity_type_id: workflowType.id, name: "测试主流程" };
    vi.mocked(api.entityTypes).mockResolvedValue({ items: [workflowType] });
    vi.mocked(api.entities).mockResolvedValue({ items: [workflowEntity] });
    vi.mocked(api.attributes).mockResolvedValue({
      items: [{ ...roleAttribute, id: "2", attribute_key: "owner", name: "负责人" }],
    });
    vi.mocked(api.relationships).mockResolvedValue({ items: [] });
    vi.mocked(api.relationshipTypes).mockResolvedValue({ items: [] });
    vi.mocked(api.entity).mockResolvedValue({
      item: workflowEntity,
      structured_attributes: [{ attribute_definition_id: "2", value: "张三", revision: 1 }],
    });
    await openEntityDetail();
    fireEvent.click(await entityDrawer().findByRole("tab", { name: "普通属性" }));
    expect((await entityDrawer().findAllByText(/普通属性/)).length).toBeGreaterThan(0);
    expect(entityDrawer().getByRole("textbox", { name: "编辑 负责人" })).toHaveValue("张三");
    expect(entityDrawer().getByRole("textbox", { name: "编辑 负责人" })).toBeEnabled();
    expect(entityDrawer().getByRole("button", { name: "提交" })).toBeDisabled();
  });

  it("类型详情抽屉属性Tab展示属性定义并可新增", async () => {
    baseMocks();
    await openTypeDetail();
    const drawer = typeDrawer();
    fireEvent.click(drawer.getByRole("tab", { name: /属性/ }));
    expect(await drawer.findByText("系统角色")).toBeInTheDocument();
    expect(drawer.getByRole("button", { name: /新增属性/ })).toBeInTheDocument();
    expect(drawer.getAllByRole("button", { name: "编辑" })[0]).toBeInTheDocument();
  });

  it("Action Tab 展示来源与支持的 Action", async () => {
    baseMocks();
    await openTypeDetail();
    const drawer = typeDrawer();
    fireEvent.click(drawer.getByRole("tab", { name: /Action/ }));
    expect(drawer.queryByText(/source-contract/)).not.toBeInTheDocument();
    expect((await drawer.findAllByText(/扩容/)).length).toBeGreaterThan(0);
    expect((await drawer.findAllByText(/发布/)).length).toBeGreaterThan(0);
  });

  it("关系 Tab 展示与该类型实体关联的关系", async () => {
    baseMocks();
    await openTypeDetail();
    const drawer = typeDrawer();
    fireEvent.click(drawer.getByRole("tab", { name: /关系/ }));
    expect(await drawer.findByText("依赖关系")).toBeInTheDocument();
    expect(drawer.getAllByText("opencoder-cli").length).toBeGreaterThan(0);
    expect(drawer.getByText(/依赖外部系统/)).toBeInTheDocument();
  });
});
