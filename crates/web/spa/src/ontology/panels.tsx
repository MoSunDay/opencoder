import { Alert, Button, Empty, Select, Space, Spin } from "antd";
import { useCallback, useEffect, useRef, useState } from "react";
import type { ComponentType } from "react";
import { useLocalStorage } from "usehooks-ts";
import { useStore } from "../store.js";
import { PageShell } from "../shell/pageShell.jsx";
import { api } from "./api";
import { EnvContext } from "./env";
import type { Environment } from "./types";
import GraphPage from "./pages/graph/GraphPage";
import EntitiesPage from "./pages/EntitiesPage";
import EntityTypesPage from "./pages/EntityTypesPage";
import RelationshipTypesPage from "./pages/RelationshipTypesPage";
import EnvironmentsPage from "./pages/EnvironmentsPage";
import { useDraftGuard } from "./navigation/DraftGuard";
import "./styles.css";

function OntologyPanel({ page, Component }: { page: string; Component: ComponentType }) {
  const { identity } = useStore();
  const guard = useDraftGuard();
  const [storedEnv, setEnv] = useLocalStorage("oc_ontology_env", "debug");
  const [environments, setEnvironments] = useState<Environment[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const request = useRef(0);
  const refreshEnvironments = useCallback(async () => {
    const current = ++request.current;
    setLoading(true); setError("");
    try {
      const { items } = await api.environments();
      if (current === request.current) setEnvironments(items.filter((item) => !item.is_deleted && item.initialization_status === "ready"));
    } catch (failure) {
      if (current === request.current) setError(failure instanceof Error ? failure.message : "环境加载失败");
    } finally { if (current === request.current) setLoading(false); }
  }, []);
  useEffect(() => { void refreshEnvironments(); return () => { request.current += 1; }; }, [refreshEnvironments]);
  const env = environments.some((item) => item.env_key === storedEnv) ? storedEnv : environments[0]?.env_key;
  const changeEnv = (next: string) => { if (next !== env) guard.run(() => setEnv(next)); };
  const context = { env: env ?? "debug", environments, setEnv: changeEnv, refreshEnvironments, canManage: identity?.role === "admin" };
  return <PageShell page={page} extra={<Space><span>环境</span><Select aria-label="Ontology 环境" value={env} loading={loading}
    style={{ minWidth: 140 }} onChange={changeEnv} options={environments.map((item) => ({ value: item.env_key, label: item.name }))} /></Space>}>
    {error ? <Alert title={error} type="error" showIcon action={<Button onClick={() => void refreshEnvironments()}>重试</Button>} />
      : loading ? <Spin /> : !env ? <Empty description="暂无可用环境" />
        : <EnvContext.Provider value={context}><Component key={env} /></EnvContext.Provider>}
  </PageShell>;
}
export const OntologyGraphPanel = () => <OntologyPanel page="ontologyGraph" Component={GraphPage} />;
export const OntologyEntitiesPanel = () => <OntologyPanel page="ontologyEntities" Component={EntitiesPage} />;
export const OntologyTypesPanel = () => <OntologyPanel page="ontologyTypes" Component={EntityTypesPage} />;
export const OntologyRelationshipsPanel = () => <OntologyPanel page="ontologyRelationships" Component={RelationshipTypesPage} />;
export const OntologyEnvironmentsPanel = () => <OntologyPanel page="ontologyEnvironments" Component={EnvironmentsPage} />;
