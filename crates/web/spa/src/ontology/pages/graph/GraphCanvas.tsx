import { FlowDirectionGraph } from "@ant-design/graphs";
import type { Graph } from "@antv/g6";
import { EdgeEvent, GraphEvent, NodeEvent } from "@antv/g6";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { GraphData } from "../../types";
import { coordinateGraphLifecycle } from "./graphLifecycle";

type ElementClickEvent = { target: { id: string } };
type Props = {
  data: GraphData;
  centerIds: string[];
  relationshipTypeNames: Record<string, string>;
  onNodeClick: (id: string) => void;
  onEdgeClick: (id: string) => void;
};

export default function GraphCanvas({
  data,
  centerIds,
  relationshipTypeNames,
  onNodeClick,
  onEdgeClick,
}: Props) {
  const graphRef = useRef<Graph>();
  const containerRef = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(620);
  const fitToViewport = useCallback(() => {
    if (!containerRef.current) return;
    const remaining = window.innerHeight - containerRef.current.getBoundingClientRect().top - 24;
    setHeight(Math.max(360, Math.floor(remaining)));
  }, []);
  useLayoutEffect(() => { fitToViewport(); });
  useEffect(() => {
    window.addEventListener("resize", fitToViewport);
    return () => window.removeEventListener("resize", fitToViewport);
  }, [fitToViewport]);
  const nodeClickRef = useRef(onNodeClick);
  const edgeClickRef = useRef(onEdgeClick);
  nodeClickRef.current = onNodeClick;
  edgeClickRef.current = onEdgeClick;

  const graphData = useMemo(
    () => ({
      nodes: data.nodes.map((node) => ({ id: node.id, data: { name: node.name } })),
      edges: data.edges.map((edge) => ({
        id: edge.id,
        source: edge.source_entity_id,
        target: edge.target_entity_id,
        data: { name: relationshipTypeNames[edge.relationship_type_id] || "关系" },
      })),
    }),
    [data, relationshipTypeNames],
  );

  const selectedState = Object.fromEntries(data.nodes.map((node) => [
    node.id, centerIds.includes(node.id) ? ["selected"] : [],
  ]));
  const selectedStateRef = useRef(selectedState);
  selectedStateRef.current = selectedState;
  useEffect(() => {
    if (graphRef.current && !graphRef.current.destroyed) void graphRef.current.setElementState(selectedStateRef.current, false);
  }, [centerIds, data.nodes]);

  return <div ref={containerRef} style={{ width: "100%", height }}>
    <FlowDirectionGraph
      onInit={coordinateGraphLifecycle}
      data={graphData}
      height={height}
      containerStyle={{ width: "100%", height: "100%" }}
      autoFit="view"
      labelField={(node) => String(node.data?.name || node.id)}
      node={{ style: { size: [220, 72] } }}
      edge={{
        style: {
          endArrow: true,
          labelText: (edge) => String(edge.data?.name || "关系"),
          labelBackground: true,
        },
      }}
      behaviors={(behaviors) => [...behaviors, "hover-activate-neighbors"]}
      onReady={(graph) => {
        // Graphin's asynchronous render callback may arrive after unmount/StrictMode cleanup.
        if (graph.destroyed) return;
        if (graphRef.current !== graph) {
          graphRef.current = graph;
          graph.on(NodeEvent.CLICK, (event) =>
            nodeClickRef.current(String((event as ElementClickEvent).target.id)),
          );
          graph.on(EdgeEvent.CLICK, (event) =>
            edgeClickRef.current(String((event as ElementClickEvent).target.id)),
          );
          graph.on(GraphEvent.AFTER_SIZE_CHANGE, () => {
            if (!graph.destroyed) void graph.fitView(undefined, false);
          });
        }
        void graph.setElementState(selectedStateRef.current, false);
      }}
      onDestroy={() => {
        graphRef.current = undefined;
      }}
    />
  </div>;
}
