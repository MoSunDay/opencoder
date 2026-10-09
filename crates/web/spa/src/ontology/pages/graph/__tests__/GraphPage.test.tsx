// @vitest-environment jsdom
import "../../../testSetup";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { apiMock, graphHandlers, graphMock, entityTypes, entities, relationshipTypes, graphNodes, aspect, edge, pageElement, finishSelection, expandFilters, renderPage, chooseScope } from "./pageFixture";
describe("GraphPage", () => {


  it("follows the type-first observation flow with a required relationship type", async () => {
    await renderPage();
    expect(apiMock.graph).not.toHaveBeenCalled();
    await chooseScope();
    expect(await screen.findByTestId("node-b")).toHaveTextContent("实体 B 完整名称");

    fireEvent.mouseDown(screen.getByRole("combobox", { name: "上游跳数" }));
    fireEvent.click(await screen.findByText("2 跳"));
    finishSelection();
    await waitFor(() => expect(apiMock.graph).toHaveBeenCalledWith("debug", {
      entityTypeIds: ["service"],
      centerIds: ["a"],
      upstreamDepth: 2,
      downstreamDepth: 3,
      relationshipTypeIds: ["depends"],
    }));
  });

  it("lets debug start from an entity without selecting a relationship type", async () => {
    render(pageElement(true, "debug"));
    fireEvent.click(screen.getByRole("checkbox", { name: "展开跨类型邻居" }));
    fireEvent.click(screen.getByRole("tab", { name: "自定义观测" }));
    await expandFilters();
    await screen.findByText("请先选择实体类型");
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体类型多选" }));
    fireEvent.click(await screen.findByText("服务类型"));
    finishSelection();
    fireEvent.keyDown(screen.getByRole("combobox", { name: "实体类型多选" }), { key: "Escape" });
    await waitFor(() => expect(screen.getByRole("combobox", { name: "实体多选" })).toBeEnabled());
    expect(screen.getByRole("button", { name: "保存切面" })).toBeEnabled();
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体多选" }));
    fireEvent.click(await screen.findByText("实体 A 完整名称"));
    finishSelection();
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 3, downstreamDepth: 3,
      expandNeighbors: true,
    }));
  });

  it("explains when selected debug centers have no path in the visible scope", async () => {
    const root = { ...entities[0], id: "root", name: "目录", entity_type_id: "directory" };
    const containment = ["a", "b"].map((target_entity_id) => ({
      ...edge, id: `contains-${target_entity_id}`, relationship_type_id: "contains", source_entity_id: "root", target_entity_id,
    }));
    apiMock.relationshipTypes.mockResolvedValue({ items: [...relationshipTypes, {
      ...relationshipTypes[0], id: "contains", type_key: "contains", name: "包含", is_directory_membership: true,
    }] });
    apiMock.graph.mockResolvedValue({ nodes: [...graphNodes, root], edges: containment, available_relationship_type_ids: ["contains"] });
    render(pageElement(true, "debug"));
    fireEvent.click(screen.getByRole("checkbox", { name: "展开跨类型邻居" }));
    fireEvent.click(screen.getByRole("tab", { name: "自定义观测" }));
    await expandFilters();
    fireEvent.mouseDown(await screen.findByRole("combobox", { name: "实体类型多选" }));
    fireEvent.click(await screen.findByText("服务类型"));
    finishSelection();
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体多选" }));
    fireEvent.click(await screen.findByText("实体 A 完整名称"));
    fireEvent.click(await screen.findByText("实体 B 完整名称"));
    finishSelection();
    expect(await screen.findByText(/所选实体在当前关系筛选和跳数内没有已确认路径/)).toBeInTheDocument();
  });

  it("lists only entities of the chosen types as centers", async () => {
    await renderPage();
    await screen.findByText("请先选择实体类型，再选择关系类型");
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体类型多选" }));
    fireEvent.click(await screen.findByText("服务类型"));
    finishSelection();
    expect(screen.getByRole("combobox", { name: "实体多选" })).toBeDisabled();
    fireEvent.keyDown(screen.getByRole("combobox", { name: "实体类型多选" }), { key: "Escape" });
    await waitFor(() => expect(screen.getByRole("combobox", { name: "关系类型多选" })).toBeEnabled());
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "关系类型多选" }));
    fireEvent.click(await screen.findByText("依赖（服务类型→服务类型）"));
    finishSelection();
    await waitFor(() => expect(screen.getByRole("combobox", { name: "实体多选" })).toBeEnabled());
    fireEvent.keyDown(screen.getByRole("combobox", { name: "关系类型多选" }), { key: "Escape" });
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体多选" }));
    expect(await screen.findByText("实体 A 完整名称")).toBeInTheDocument();
    expect(screen.queryByText("其他类型实体")).not.toBeInTheDocument();
  });

  it("cascades relation type candidates and labels their endpoints", async () => {
    await renderPage();
    await chooseScope();
    const relationSelect = screen.getByRole("combobox", { name: "关系类型多选" }).closest(".ant-select");
    expect(relationSelect).not.toBeNull();
    await waitFor(() => expect(within(relationSelect as HTMLElement).getAllByText("依赖（服务类型→服务类型）").length).toBeGreaterThan(0));
    const visibleOption = () => document.querySelector('.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option[title="依赖（服务类型→服务类型）"]');
    const clickOption = async () => {
      await waitFor(() => expect(screen.getByRole("combobox", { name: "关系类型多选" })).toBeEnabled());
      if (!visibleOption()) fireEvent.mouseDown(screen.getByRole("combobox", { name: "关系类型多选" }));
      const option = await waitFor(() => {
        const found = visibleOption();
        expect(found).not.toBeNull();
        return found as HTMLElement;
      });
      expect(screen.queryByText(/未使用关系/)).not.toBeInTheDocument();
      fireEvent.click(option);
      finishSelection();
    };
    await clickOption();
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 3, downstreamDepth: 3,
    }));
    await clickOption();
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 3, downstreamDepth: 3, relationshipTypeIds: ["depends"],
    }));
  });

  it("keeps the two aspect tabs and shared drawer maintenance", async () => {
    await renderPage();
    await chooseScope();
    expect(screen.getByRole("tab", { name: "自定义观测" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "切面观测" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /创建关系/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("combobox", { name: "待固定关系" })).not.toBeInTheDocument();
    expect(screen.queryByText("固定观测")).not.toBeInTheDocument();
    fireEvent.click(await screen.findByTestId("node-a"));
    expect(screen.getByText("entity-drawer-a")).toBeInTheDocument();
    fireEvent.click(screen.getByText("edge-edge"));
    expect(screen.getByText("relationship-drawer-edge")).toBeInTheDocument();
  });

  it("clears the open drawers when switching between aspect tabs", async () => {
    await renderPage();
    await chooseScope();
    fireEvent.click(await screen.findByTestId("node-a"));
    expect(screen.getByText("entity-drawer-a")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "切面观测" }));
    await expandFilters();
    expect(await screen.findByText("请选择数据切面")).toBeInTheDocument();
    expect(screen.queryByText("entity-drawer-a")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: "自定义观测" }));
    await expandFilters();
    expect(await screen.findByTestId("node-a")).toBeInTheDocument();
    expect(screen.queryByText("entity-drawer-a")).not.toBeInTheDocument();
  });

  it("hides the save-aspect entry for read-only users", async () => {
    await renderPage(false);
    await screen.findByText("请先选择实体类型，再选择关系类型");
    expect(screen.queryByRole("button", { name: "保存切面" })).toBeNull();
  });

  it("restores the successful custom observation including cross-type neighbors", async () => {
    const page = await renderPage();
    await chooseScope();
    fireEvent.click(screen.getByRole("checkbox", { name: "展开跨类型邻居" }));
    await waitFor(() => expect(JSON.parse(localStorage.getItem("oc_ontology_observation:debug")!).expandNeighbors).toBe(true));
    page.unmount();
    apiMock.graph.mockClear();
    render(pageElement());
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], relationshipTypeIds: ["depends"],
      upstreamDepth: 3, downstreamDepth: 3, expandNeighbors: true,
    }));
    expect(screen.getByRole("tab", { name: "自定义观测" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("checkbox", { name: "展开跨类型邻居" })).toBeChecked();
  });

  it("saves the chosen scope as a new named aspect", async () => {
    await renderPage();
    await screen.findByText("请先选择实体类型，再选择关系类型");
    expect(screen.getByRole("button", { name: "保存切面" })).toBeDisabled();
    await chooseScope();
    fireEvent.click(screen.getByRole("button", { name: "保存切面" }));
    fireEvent.change(await screen.findByLabelText("名称"), { target: { value: "核心服务切面" } });
    fireEvent.change(screen.getByLabelText("描述"), { target: { value: "核心服务观测范围" } });
    fireEvent.click(screen.getByRole("button", { name: /^保\s*存$/ }));
    await waitFor(() => expect(apiMock.createGraphAspect).toHaveBeenCalledWith("debug", {
      name: "核心服务切面",
      description: "核心服务观测范围",
      entity_type_ids: ["service"],
      relationship_type_ids: ["depends"],
      default_center_ids: ["a"],
      default_upstream_depth: 3,
      default_downstream_depth: 3,
    }));
    expect(await screen.findByText("切面已保存")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => expect(apiMock.graphAspects).toHaveBeenCalledTimes(2));
  });

  it("overwrites an existing aspect from the save modal", async () => {
    await renderPage();
    await chooseScope();
    apiMock.graphAspects.mockResolvedValue({ items: [aspect] });
    fireEvent.click(screen.getByRole("button", { name: "刷新当前观测" }));
    await waitFor(() => expect(apiMock.graphAspects).toHaveBeenCalledTimes(2));
    fireEvent.click(screen.getByRole("button", { name: "保存切面" }));
    fireEvent.click(await screen.findByText("更新已有切面"));
    finishSelection();
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "已有切面选择" }));
    fireEvent.click(await screen.findByText("核心服务切面"));
    finishSelection();
    expect(screen.getByLabelText("名称")).toHaveValue("核心服务切面");
    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "覆盖后的切面" } });
    fireEvent.click(screen.getByRole("button", { name: /^保\s*存$/ }));
    await waitFor(() => expect(apiMock.updateGraphAspect).toHaveBeenCalledWith("debug", "aspect-1", {
      name: "覆盖后的切面",
      description: "核心服务观测范围",
      entity_type_ids: ["service"],
      relationship_type_ids: ["depends"],
      default_center_ids: ["a"],
      default_upstream_depth: 3,
      default_downstream_depth: 3,
      is_deleted: false,
      expected_revision: 1,
    }));
    expect(await screen.findByText("切面已保存")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  }, 10000);

  it("deletes an existing aspect from the save modal", async () => {
    await renderPage();
    await chooseScope();
    apiMock.graphAspects.mockResolvedValue({ items: [aspect] });
    fireEvent.click(screen.getByRole("button", { name: "刷新当前观测" }));
    await waitFor(() => expect(apiMock.graphAspects).toHaveBeenCalledTimes(2));
    fireEvent.click(await screen.findByRole("button", { name: "保存切面" }));
    fireEvent.click(await screen.findByText("更新已有切面"));
    finishSelection();
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "已有切面选择" }));
    fireEvent.click(await screen.findByText("核心服务切面"));
    finishSelection();
    fireEvent.click(screen.getByRole("button", { name: "删除该切面" }));
    fireEvent.click(await screen.findByRole("button", { name: /^删\s*除$/ }));
    await waitFor(() => expect(apiMock.deleteGraphAspect).toHaveBeenCalledWith("debug", "aspect-1", 1));
    expect(await screen.findByText("切面已删除")).toBeInTheDocument();
  }, 10000);


});
