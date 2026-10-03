import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../../api";
import type { GraphAspect } from "../../../types";

/** 创建切面的请求体；aspect_key 由后端生成，前端不传。 */
export type GraphAspectInput = { name: string; description?: string; entity_type_ids: string[]; relationship_type_ids: string[]; default_center_ids: string[]; default_upstream_depth: number | null; default_downstream_depth: number | null };
/** 更新切面的请求体，携带乐观锁修订号与删除标记。 */
export type GraphAspectPatch = GraphAspectInput & { is_deleted?: boolean; expected_revision: number };

/** 按 ENV 维护切面列表：loading/error/reload 与 create/update/delete 包装（成功后刷新列表）。 */
export function useGraphAspects(env: string) {
  const [aspects, setAspects] = useState<GraphAspect[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const requestRef = useRef(0);
  const mountedRef = useRef(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; requestRef.current += 1; };
  }, []);

  const reload = useCallback(async () => {
    if (!mountedRef.current) return;
    const request = ++requestRef.current;
    const current = () => mountedRef.current && request === requestRef.current;
    setLoading(true);
    setError("");
    try {
      const response = await api.graphAspects(env);
      if (!current()) return;
      setAspects(response.items.filter((item) => !item.is_deleted));
    } catch (reason) {
      if (!current()) return;
      setAspects([]);
      setError(reason instanceof Error ? reason.message : "切面加载失败");
    } finally {
      if (current()) setLoading(false);
    }
  }, [env]);

  useEffect(() => {
    void reload();
    return () => { requestRef.current += 1; };
  }, [reload]);

  const create = useCallback(async (value: GraphAspectInput) => {
    const { item } = await api.createGraphAspect(env, value);
    await reload();
    return item;
  }, [env, reload]);

  const update = useCallback(async (id: string, value: GraphAspectPatch) => {
    const { item } = await api.updateGraphAspect(env, id, value);
    await reload();
    return item;
  }, [env, reload]);

  const remove = useCallback(async (id: string, expectedRevision: number) => {
    const { item } = await api.deleteGraphAspect(env, id, expectedRevision);
    await reload();
    return item;
  }, [env, reload]);

  return { aspects, loading, error, reload, create, update, remove };
}
