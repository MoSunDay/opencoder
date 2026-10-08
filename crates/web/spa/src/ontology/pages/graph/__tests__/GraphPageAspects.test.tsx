// @vitest-environment jsdom
import "../../../testSetup";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { apiMock, graphHandlers, graphMock, entityTypes, entities, relationshipTypes, graphNodes, aspect, edge, pageElement, finishSelection, expandFilters, renderPage, chooseScope } from "./pageFixture";
describe("GraphPage saved observation", () => {
  it("observes a saved aspect at three hops without requiring centers", async () => {
    apiMock.graphAspects.mockResolvedValue({ items: [aspect] });
    await renderPage();
    fireEvent.click(await screen.findByRole("tab", { name: "切面观测" }));
    await expandFilters();
    expect(screen.getByRole("button", { name: "切面说明" })).toBeEnabled();
    expect(await screen.findByRole("combobox", { name: "切面观测实体多选" })).toBeEnabled();
    expect(screen.getByRole("combobox", { name: "切面观测实体多选" })).toBeEnabled();
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], relationshipTypeIds: ["depends"], upstreamDepth: 3, downstreamDepth: 3,
    }));
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "切面下游跳数" }));
    const downstreamOption = document.querySelector('.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option[title="1 跳"]');
    fireEvent.click(downstreamOption!);
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], relationshipTypeIds: ["depends"], upstreamDepth: 3, downstreamDepth: 1,
    }));
    await screen.findByTestId("node-a");
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "切面观测实体多选" }));
    const aspectCenterOption = await waitFor(() => {
      const found = document.querySelector('.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option[title="实体 A 完整名称"]');
      expect(found).not.toBeNull();
      return found as HTMLElement;
    });
    fireEvent.click(aspectCenterOption);
    finishSelection();
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], relationshipTypeIds: ["depends"], centerIds: ["a"], upstreamDepth: 3, downstreamDepth: 1,
    }));
  });

  it("opens an aspect with its saved center and hop defaults", async () => {
    apiMock.graphAspects.mockResolvedValue({ items: [{ ...aspect,
      default_center_ids: ["a"], default_upstream_depth: 0, default_downstream_depth: 2,
    }] });
    await renderPage();
    fireEvent.click(await screen.findByRole("tab", { name: "切面观测" }));
    await expandFilters();
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], relationshipTypeIds: ["depends"], centerIds: ["a"],
      upstreamDepth: 0, downstreamDepth: 2,
    }));
  });

  it("treats saved relationships as an editable observation filter", async () => {
    apiMock.graphAspects.mockResolvedValue({ items: [aspect] });
    await renderPage();
    fireEvent.click(await screen.findByRole("tab", { name: "切面观测" }));
    await expandFilters();
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], relationshipTypeIds: ["depends"], upstreamDepth: 3, downstreamDepth: 3,
    }));
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "切面关系类型多选" }));
    const option = await waitFor(() => {
      const found = document.querySelector('.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option[title^="未使用关系"]');
      expect(found).not.toBeNull();
      return found as HTMLElement;
    });
    fireEvent.click(option);
      finishSelection();
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], relationshipTypeIds: ["depends", "unused"], upstreamDepth: 3, downstreamDepth: 3,
    }));
    expect(apiMock.updateGraphAspect).not.toHaveBeenCalled();
  });

  it("selects and highlights multiple centers", async () => {
    await renderPage();
    await chooseScope();
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体多选" }));
    const option = document.querySelector('.ant-select-item-option[title="实体 B 完整名称"]');
    fireEvent.click(option!);
    finishSelection();
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a", "b"], upstreamDepth: 3, downstreamDepth: 3, relationshipTypeIds: ["depends"],
    }));
    await screen.findByTestId("node-b");
    expect(graphMock.setElementState).toHaveBeenLastCalledWith({ a: [], b: [], edge: [] }, false);
  });

  it("shows failures with a working retry", async () => {
    apiMock.entities.mockRejectedValueOnce(new Error("实体查询失败"));
    await renderPage();
    expect(await screen.findByText("实体查询失败")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /重\s*试/ }));
    await screen.findByText("请先选择实体类型，再选择关系类型");
    expect(screen.queryByText("实体查询失败")).not.toBeInTheDocument();
    expect(apiMock.entities).toHaveBeenCalledTimes(2);
  });

  it("resets observation on ENV changes and ignores the previous ENV response", async () => {
    const page = await renderPage();
    await chooseScope();
    let resolveOld!: (value: unknown) => void;
    apiMock.graph.mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }));
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "上游跳数" }));
    fireEvent.click(await screen.findByText("2 跳"));
    finishSelection();
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 2, downstreamDepth: 3, relationshipTypeIds: ["depends"],
    }));
    apiMock.entities.mockResolvedValue({ items: [{ ...entities[0], id: "new", name: "新环境实体" }] });
    page.rerender(pageElement(true, "new-env"));
    await expandFilters();
    await screen.findByText("请先选择实体类型，再选择关系类型");
    await act(async () => { resolveOld({ nodes: graphNodes, edges: [edge], available_relationship_type_ids: ["depends"] }); });
    expect(screen.queryByTestId("node-a")).not.toBeInTheDocument();
    fireEvent.mouseDown(screen.getByRole("combobox", { name: "实体类型多选" }));
    expect(await screen.findByText("服务类型")).toBeInTheDocument();
    expect(apiMock.entities).toHaveBeenLastCalledWith("new-env");
  });

});
