import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../../api";
import { useEnv } from "../../../env";
import type { GraphAspect, GraphResponse } from "../../../types";

const EMPTY_GRAPH: GraphResponse = { nodes: [], edges: [], available_relationship_type_ids: [] };

export type AspectObservationInput = {
  /** 已保存切面；实体类型固定，关系类型可在本次观测中调整。 */
  facet?: GraphAspect;
  expandNeighbors?: boolean;
  relationshipTypeIds: string[];
  /** 观测实体；为空时观察整个切面范围（不传 center）。 */
  centerIds: string[];
  upstreamDepth: number;
  downstreamDepth: number;
};

/** 切面观测：选择切面后用默认或手选跳数发起请求；请求序号与卸载守卫避免旧结果覆盖。 */
export function useAspectObservation({ facet, relationshipTypeIds, centerIds, upstreamDepth, downstreamDepth, expandNeighbors = false }: AspectObservationInput) {
  const { env } = useEnv();
  const [data, setData] = useState<GraphResponse>(EMPTY_GRAPH);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const requestRef = useRef(0);
  const mountedRef = useRef(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; requestRef.current += 1; };
  }, []);

  const load = useCallback(async () => {
    if (!mountedRef.current) return;
    if (!facet) {
      requestRef.current += 1;
      setData(EMPTY_GRAPH);
      setLoading(false);
      setError("");
      return;
    }
    const request = ++requestRef.current;
    const current = () => mountedRef.current && request === requestRef.current;
    setLoading(true);
    setError("");
    try {
      const nextGraph = await api.graph(env, {
        entityTypeIds: facet.entity_type_ids,
        ...(relationshipTypeIds.length ? { relationshipTypeIds } : {}),
        ...(expandNeighbors ? { expandNeighbors: true } : {}),
        ...(centerIds.length ? { centerIds } : {}),
        upstreamDepth,
        downstreamDepth,
      });
      if (!current()) return;
      setData(nextGraph);
    } catch (reason) {
      if (!current()) return;
      setData(EMPTY_GRAPH);
      setError(reason instanceof Error ? reason.message : "拓扑加载失败");
    } finally {
      if (current()) setLoading(false);
    }
  }, [env, facet, relationshipTypeIds, centerIds, upstreamDepth, downstreamDepth, expandNeighbors]);

  useEffect(() => {
    void load();
    return () => { requestRef.current += 1; };
  }, [load]);

  const reload = useCallback(async () => {
    await load();
  }, [load]);

  return { data, loading, error, reload };
}
