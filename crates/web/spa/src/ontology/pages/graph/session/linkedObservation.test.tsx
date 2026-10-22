// @vitest-environment jsdom
import "../../../testSetup";
import { screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { apiMock, aspect, entities, renderPage } from "../__tests__/pageFixture";
import { linkedObservation } from "./linkedObservation";

afterEach(() => window.history.replaceState(null, "", "/"));

it("opens a linked aspect and center ahead of previous browser observation", async () => {
  localStorage.setItem("oc_ontology_observation:debug", JSON.stringify({ mode: "aspect-test", selection: {
    entityTypeIds: ["service"], relationshipTypeIds: [], centerIds: ["a"], upstreamDepth: 3, downstreamDepth: 3,
  } }));
  apiMock.graphAspects.mockResolvedValue({ items: [aspect] });
  window.history.replaceState(null, "", "/#ontology_aspect=core-service&ontology_center=b&ontology_upstream=0&ontology_downstream=2");
  await renderPage();
  await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
    expandNeighbors: true, entityTypeIds: ["service"], relationshipTypeIds: ["depends"], centerIds: ["b"], upstreamDepth: 0, downstreamDepth: 2,
  }));
  expect(await screen.findByTestId("node-b")).toHaveAttribute("data-center", "true");
  expect(apiMock.updateGraphAspect).not.toHaveBeenCalled();
});

it("reports unavailable linked entities and restores an available aspect", async () => {
  apiMock.graphAspects.mockResolvedValue({ items: [aspect] });
  window.history.replaceState(null, "", "/#ontology_aspect=core-service&ontology_center=missing");
  await renderPage();
  expect(await screen.findByText("链接中的观测点或切面已不可用，已恢复可用观测范围。")).toBeInTheDocument();
  await waitFor(() => expect(apiMock.graph).toHaveBeenCalledWith("debug", {
    expandNeighbors: true, entityTypeIds: ["service"], relationshipTypeIds: ["depends"], upstreamDepth: 3, downstreamDepth: 3,
  }));
});

it("resolves standalone centers and rejects centers outside a named aspect", () => {
  expect(linkedObservation({ centerId: "b" }, entities, [aspect])).toMatchObject({ mode: "aspect-test", selection: {
    entityTypeIds: ["service"], centerIds: ["b"], relationshipTypeIds: [], upstreamDepth: 1, downstreamDepth: 1,
  } });
  expect(linkedObservation({ aspectKey: aspect.aspect_key, centerId: "c" }, entities, [aspect])).toBeUndefined();
});
