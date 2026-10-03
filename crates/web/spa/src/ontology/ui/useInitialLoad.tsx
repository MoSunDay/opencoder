import { Alert, Button } from "antd";
import { useCallback, useEffect, useRef, useState } from "react";

export function useInitialLoad(key: string, load: () => Promise<unknown>) {
  const loader = useRef(load);
  loader.current = load;
  const request = useRef(0);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const retry = useCallback(async () => {
    const id = ++request.current;
    setError(""); setLoading(true);
    try { await loader.current(); }
    catch (failure) { if (id === request.current) setError(failure instanceof Error ? failure.message : "加载失败"); }
    finally { if (id === request.current) setLoading(false); }
  }, [key]);
  useEffect(() => { void retry(); return () => { request.current += 1; }; }, [retry]);
  return { error, loading, retry };
}
export function LoadFeedback({ status }: { status: ReturnType<typeof useInitialLoad> }) {
  return status.error ? <Alert type="error" showIcon title={status.error} style={{ marginBottom: 16 }}
    action={<Button onClick={() => void status.retry()}>重试</Button>} /> : null;
}
