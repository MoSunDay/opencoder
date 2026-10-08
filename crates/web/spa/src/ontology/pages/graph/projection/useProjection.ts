import { useMemo, useState } from "react";
import type { GraphData } from "../../../types";
import { INITIAL_EDGES, INITIAL_NODES, projectGraph } from "./model";

export function useProjection(data: GraphData, centers: string[], scope?: string) {
  const key = JSON.stringify([scope, centers, data.nodes.map((node) => node.id), data.edges.map((edge) => edge.id)]);
  const [state, setState] = useState({ key, nodeBudget: INITIAL_NODES, edgeBudget: INITIAL_EDGES, retainedNodes: [] as string[], retainedEdges: [] as string[], reveal: undefined as string | undefined });
  const options = state.key === key ? state : { nodeBudget: INITIAL_NODES, edgeBudget: INITIAL_EDGES };
  const projection = useMemo(() => projectGraph(data, centers, options), [data, key, state]);
  const expand = (reveal?: string) => setState({ key, nodeBudget: Math.max(options.nodeBudget, projection.nodeIds.length) + 20,
    edgeBudget: Math.max(options.edgeBudget, projection.edgeIds.length) + 40, retainedNodes: projection.nodeIds, retainedEdges: projection.edgeIds, reveal });
  return { ...projection, expand, key };
}
