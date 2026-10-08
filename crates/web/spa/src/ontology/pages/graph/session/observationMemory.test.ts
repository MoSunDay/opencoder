// @vitest-environment jsdom
import "../../../testSetup";
import { beforeEach, expect, it } from "vitest";
import { readObservation, writeObservation, validSelection } from "./observationMemory";
import { initialSelection } from "../observationSelection";

beforeEach(() => localStorage.clear());
it("isolates observation memory by ENV and rejects malformed state", () => {
  const selection = { ...initialSelection(), entityTypeIds: ["type"], centerIds: ["node"] };
  writeObservation("jy-hub", { mode: "aspect-test", selection });
  expect(readObservation("jy-hub")?.selection).toEqual(selection);
  expect(readObservation("debug")).toBeUndefined();
  localStorage.setItem("oc_ontology_observation:jy-hub", JSON.stringify({ mode: "aspect-test", selection: { ...selection, upstreamDepth: 99 } }));
  expect(readObservation("jy-hub")).toBeUndefined();
});
it("does not silently broaden a restored scope after its selected records disappear", () => {
  expect(validSelection({ ...initialSelection(), entityTypeIds: ["deleted"], relationshipTypeIds: ["missing"] }, { entities: [], entityTypes: [], relationshipTypes: [] })).toBe(false);
});
