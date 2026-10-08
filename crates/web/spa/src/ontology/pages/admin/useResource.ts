import { useCallback, useEffect, useRef, useState } from "react";

/** A failed refresh retains the last successful value; stale and unmounted requests cannot publish. */
export function useResource<T>(scope: string, loader: () => Promise<T>) {
  const source = useRef(loader); source.current = loader;
  const generation = useRef(0);
  const [data, setData] = useState<T>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const reload = useCallback(async () => {
    const request = ++generation.current;
    setLoading(true); setError("");
    try {
      const result = await source.current();
      if (request === generation.current) setData(result);
    } catch (reason) {
      if (request === generation.current) setError(reason instanceof Error ? reason.message : "加载失败");
    } finally { if (request === generation.current) setLoading(false); }
  }, [scope]);
  useEffect(() => { setData(undefined); void reload(); return () => { generation.current += 1; }; }, [reload]);
  return { data, loading, error, reload };
}
