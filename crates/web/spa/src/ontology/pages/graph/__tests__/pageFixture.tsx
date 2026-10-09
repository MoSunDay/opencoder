import { ConfigProvider } from "antd";
import { App as AntdApp } from "antd";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { EnvContext } from "../../../env";
import GraphPage from "../GraphPage";
vi.setConfig({ testTimeout: 15000 });

vi.mock("antd", async (original) => { const actual = await original<typeof import("antd")>(); return { ...actual, Grid: { ...actual.Grid, useBreakpoint: () => ({ md: true, lg: true }) } }; });
const { apiMock, graphHandlers, graphMock } = vi.hoisted(() => {
  const handlers: Record<string, (event: { target: { id: string } }) => void> = {};
  return {
    graphHandlers: handlers,
    graphMock: {
      on: vi.fn((event: string, handler: (event: { target: { id: string } }) => void) => {
        handlers[event] = handler;
      }),
      setElementState: vi.fn().mockResolvedValue(undefined),
      getZoom: vi.fn(() => 1), getNodeData: vi.fn(() => [] as { id: string; data: { name: string } }[]), getEdgeData: vi.fn(() => [] as { id: string }[]),
      updateEdgeData: vi.fn(), draw: vi.fn().mockResolvedValue(undefined),
      zoomTo: vi.fn().mockResolvedValue(undefined), zoomBy: vi.fn().mockResolvedValue(undefined),
      fitView: vi.fn().mockResolvedValue(undefined), focusElement: vi.fn().mockResolvedValue(undefined),
      getElementPosition: vi.fn(() => [100, 100]), getViewportByCanvas: vi.fn(() => [100, 100]),
      getCanvasCenter: vi.fn(() => [400, 250]), translateBy: vi.fn().mockResolvedValue(undefined),
    },
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
vi.mock("@ant-design/graphs", () => ({
  FlowDirectionGraph: (props: {
    data: { nodes: Array<{ id: string; data: { name: string } }>; edges: Array<{ id: string }> };
    onReady: (graph: typeof graphMock) => void;
  }) => {
    graphMock.getNodeData.mockReturnValue(props.data.nodes);
    graphMock.getEdgeData.mockReturnValue(props.data.edges);
    props.onReady(graphMock);
    return (
      <div data-testid="mock-graph">
        {props.data.nodes.map((node) => (
          <button data-testid={`node-${node.id}`} key={node.id} onClick={() => graphHandlers["node:click"]?.({ target: { id: node.id } })}>
            {node.data.name}
          </button>
        ))}
        {props.data.edges.map((item) => (
          <button key={item.id} onClick={() => graphHandlers["edge:click"]?.({ target: { id: item.id } })}>
            edge-{item.id}
          </button>
        ))}
      </div>
    );
  },

}));
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
      <EnvContext.Provider value={{ env, environments: [], setEnv: vi.fn(), refreshEnvironments: vi.fn(), canManage }}>
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
  await screen.findByText("请先选择实体类型，再选择关系类型");
  expect(apiMock.graph).not.toHaveBeenCalled();
  expect(screen.getByRole("combobox", { name: "实体多选" })).toBeDisabled();
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体类型多选" }));
  fireEvent.click(await screen.findByText("服务类型"));
    finishSelection();
  fireEvent.keyDown(screen.getByRole("combobox", { name: "实体类型多选" }), { key: "Escape" });
  await waitFor(() => expect(screen.getByRole("combobox", { name: "关系类型多选" })).toBeEnabled());
  expect(screen.getByRole("combobox", { name: "实体多选" })).toBeDisabled();
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "关系类型多选" }));
  fireEvent.click(await screen.findByText("依赖（服务类型→服务类型）"));
    finishSelection();
  await waitFor(() => expect(screen.getByRole("combobox", { name: "实体多选" })).toBeEnabled());
  fireEvent.keyDown(screen.getByRole("combobox", { name: "关系类型多选" }), { key: "Escape" });
  fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体多选" }));
  fireEvent.click(await screen.findByText("实体 A 完整名称"));
    finishSelection();
  await waitFor(() => expect(apiMock.graph).toHaveBeenCalledWith("debug", {
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
    Object.keys(graphHandlers).forEach((key) => delete graphHandlers[key]);
    apiMock.graph.mockResolvedValue({ nodes: graphNodes, edges: [edge], available_relationship_type_ids: ["depends", "unused"] });
    apiMock.entities.mockResolvedValue({ items: entities });
    apiMock.entityTypes.mockResolvedValue({ items: entityTypes });
    apiMock.relationships.mockResolvedValue({ items: [edge] });
    apiMock.relationshipTypes.mockResolvedValue({ items: relationshipTypes });
    apiMock.graphAspects.mockResolvedValue({ items: [] });
  });
export { chooseScope, apiMock, graphHandlers, graphMock };
