import { describe, expect, it } from "vitest";
import { linkedPage } from "../../fleet/navigation/links.js";
import { observationLink, withoutObservationLink } from "./observationLink";

describe("observation links", () => {
  it("opens Ontology and gives fragment selection priority over query parameters", () => {
    expect(linkedPage("?ontology_center=old", "#ontology_aspect=components&ontology_center=node%20A&ontology_upstream=0&ontology_downstream=2")).toBe("ontologyGraph");
    expect(observationLink("?ontology_center=old", "#ontology_aspect=components&ontology_center=node%20A&ontology_upstream=0&ontology_downstream=2"))
      .toEqual({ aspectKey: "components", centerId: "node A", upstreamDepth: 0, downstreamDepth: 2 });
    expect(linkedPage("?execution=task&ontology_center=node")).toBe("topics");
  });
  it("ignores empty selections and invalid hop counts", () => {
    expect(observationLink("?ontology_center=+")).toBeNull();
    expect(observationLink("?ontology_center=node&ontology_upstream=-1&ontology_downstream=10"))
      .toEqual({ centerId: "node", aspectKey: undefined, upstreamDepth: undefined, downstreamDepth: undefined });
  });
  it("removes only observation parameters when navigating away", () => {
    expect(withoutObservationLink("http://localhost/?keep=1&ontology_center=a#ontology_aspect=b&note=kept")).toBe("/?keep=1#note=kept");
    expect(withoutObservationLink("http://localhost/?ontology_center=a#section")).toBe("/#section");
  });
});
