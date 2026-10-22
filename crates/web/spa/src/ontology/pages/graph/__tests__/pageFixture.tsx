import { ConfigProvider } from "antd";
import { App as AntdApp } from "antd";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, vi } from "vitest";
import { EnvContext } from "../../../env";
import GraphPage from "../GraphPage";
vi.setConfig({ testTimeout: 15000 });

vi.mock("antd", async (original) => { const actual = await original<typeof import("antd")>(); return { ...actual, Grid: { ...actual.Grid, useBreakpoint: () => ({ md: true, lg: true }) } }; });
const { apiMock } = vi.hoisted(() => {
  return {
    apiMock: {
      graph: vi.fn(),
      entities: vi.fn(),
      entityTypes: vi.fn(),
      relationships: vi.fn(),
      relationshipTypes: vi.fn(),
      graphAspects: vi.fn(),
      createGraphAspect: vi.fn().mockResolvedValue({ item: {} }),
      updateGraphAspect: vi.fn().mockResolvedValue({ item: {} }),
      deleteGraphAspect: vi.fn().mockResolvedValue({ item: {} }),
      createRelationship: vi.fn().mockResolvedValue({ item: {} }),
      updateRelationship: vi.fn().mockResolvedValue({ item: {} }),
    },
  };
});

vi.mock("../../../api", () => ({ api: apiMock }));
vi.mock("../canvas3d/GraphScene", async () => {
  const { forwardRef } = await import("react");
  return { default: forwardRef((props: import("../canvas3d/GraphScene").SceneProps, _ref) => <div data-testid="mock-graph">
    {props.data.nodes.map((node) => <button data-testid={`node-${node.id}`} key={node.id}
      data-center={props.centerIds.includes(node.id)} data-selected={node.id === props.selectedNodeId}
      onClick={() => props.onNodeClick(node.id)}>{node.name}</button>)}
    {props.data.edges.map((edge) => <button key={edge.id} onClick={() => props.onEdgeClick(edge.id)}>edge-{edge.id}</button>)}
  </div>) };
});
vi.mock("../EntityDrawer", () => ({
  default: ({ entity }: { entity?: { id: string } }) => <div>entity-drawer-{entity?.id || "closed"}</div>,
}));
vi.mock("../RelationshipDrawer", () => ({
  default: ({ relationship }: { relationship?: { id: string } }) => <div>relationship-drawer-{relationship?.id || "closed"}</div>,
}));

const entityTypes = [{ id: "service", env_num: 1, type_key: "service", name: "服务类型", description: "", is_system: false, revision: 1, is_deleted: false }];
const entities = [
  { id: "a", env_num: 1, entity_type_id: "service", name: "实体 A 完整名称", description: "", revision: 1, is_deleted: false },
  { id: "b", env_num: 1, entity_type_id: "service", name: "实体 B 完整名称", description: "", revision: 1, is_deleted: false },
  { id: "c", env_num: 1, entity_type_id: "other", name: "其他类型实体", description: "", revision: 1, is_deleted: false },
];
const relationshipTypes = [
  {
    id: "depends", env_num: 1, type_key: "depends", name: "依赖", description: "", is_directory_membership: false, is_system: false,
    source_entity_type_id: "service", target_entity_type_ids: ["service"], revision: 1, is_deleted: false,
  },
  {
    id: "unused", env_num: 1, type_key: "unused", name: "未使用关系", description: "", is_directory_membership: false, is_system: false,
    source_entity_type_id: "other", target_entity_type_ids: ["other"], revision: 1, is_deleted: false,
  },
];
const graphNodes = entities.filter((item) => item.id !== "c");
const aspect = {
  id: "aspect-1",
  env_num: 1,
  aspect_key: "core-service",
  name: "核心服务切面",
  description: "核心服务观测范围",
  entity_type_ids: ["service"],
  relationship_type_ids: ["depends"],
  default_center_ids: [],
  default_upstream_depth: null,
  default_downstream_depth: null,
  revision: 1,
  is_deleted: false,
};
const edge = {
  id: "edge",
  env_num: 1,
  relationship_type_id: "depends",
  source_entity_id: "a",
  target_entity_id: "b",
  description: "依赖关系",
  revision: 1,
  is_deleted: false,
  is_pinned: false,
};

function pageElement(canManage = true, env = "debug") {
  return (
    <ConfigProvider theme={{ token: { motion: false } }}><AntdApp>
      <EnvContext.Provider value={{ env, canManage }}>
        <GraphPage />
      </EnvContext.Provider>
    </AntdApp></ConfigProvider>
  );
}

function finishSelection() {
  const done = document.querySelector(".ant-select-dropdown:not(.ant-select-dropdown-hidden) button");
  if (done) fireEvent.click(done);
}

async function expandFilters() {
  const buttons = within(screen.getByRole("tabpanel")).queryAllByRole("button", { name: "调整范围" });
  if (buttons[0]) fireEvent.click(buttons[0]);
  await screen.findAllByText("调整观测范围");
}
async function renderPage(canManage = true) { const page = render(pageElement(canManage)); await expandFilters(); return page; }

async function chooseScope() {
  await screen.findByText("请先选择实体类型");
  expect(apiMock.graph).not.toHaveBeenCalled();
  expect(screen.getByRole("combobox", { name: "实体多选" })).toBeDisabled();
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体类型多选" }));
  fireEvent.click(await screen.findByText("服务类型"));
    finishSelection();
  fireEvent.keyDown(screen.getByRole("combobox", { name: "实体类型多选" }), { key: "Escape" });
  await waitFor(() => expect(screen.getByRole("combobox", { name: "关系类型多选" })).toBeEnabled());
  expect(screen.getByRole("combobox", { name: "实体多选" })).toBeEnabled();
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "关系类型多选" }));
  fireEvent.click(await screen.findByText("依赖（服务类型→服务类型）"));
    finishSelection();
  await waitFor(() => expect(screen.getByRole("combobox", { name: "实体多选" })).toBeEnabled());
  fireEvent.keyDown(screen.getByRole("combobox", { name: "关系类型多选" }), { key: "Escape" });
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体多选" }));
  fireEvent.click(await screen.findByText("实体 A 完整名称"));
    finishSelection();
  await waitFor(() => expect(apiMock.graph).toHaveBeenCalledWith("debug", {
    expandNeighbors: true,
    entityTypeIds: ["service"],
    centerIds: ["a"],
    upstreamDepth: 3,
    downstreamDepth: 3,
    relationshipTypeIds: ["depends"],
  }));
  fireEvent.keyDown(screen.getByRole("combobox", { name: "实体多选" }), { key: "Escape" });
  await screen.findByTestId("node-a");
}


export { entityTypes, entities, relationshipTypes, graphNodes, aspect, edge, pageElement, finishSelection, expandFilters, renderPage };

  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    apiMock.graph.mockResolvedValue({ nodes: graphNodes, edges: [edge], available_relationship_type_ids: ["depends", "unused"] });
    apiMock.entities.mockResolvedValue({ items: entities });
    apiMock.entityTypes.mockResolvedValue({ items: entityTypes });
    apiMock.relationships.mockResolvedValue({ items: [edge] });
    apiMock.relationshipTypes.mockResolvedValue({ items: relationshipTypes });
    apiMock.graphAspects.mockResolvedValue({ items: [] });
  });
export { chooseScope, apiMock };
