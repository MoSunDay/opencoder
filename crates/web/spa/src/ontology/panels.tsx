import type { ComponentType } from "react";
import { useStore } from "../store.js";
import { PageShell } from "../shell/pageShell.jsx";
import { EnvContext, SERVER_ENV } from "./env";
import GraphPage from "./pages/graph/GraphPage";
import EntitiesPage from "./pages/EntitiesPage";
import EntityTypesPage from "./pages/EntityTypesPage";
import RelationshipTypesPage from "./pages/RelationshipTypesPage";
import "./styles.css";

function OntologyPanel({ page, Component }: { page: string; Component: ComponentType }) {
  const { identity } = useStore();
  const context = { env: SERVER_ENV, canManage: ["admin", "editor"].includes(identity?.role ?? "") };
  return <PageShell page={page}>
    <EnvContext.Provider value={context}><Component /></EnvContext.Provider>
  </PageShell>;
}
export const OntologyGraphPanel = () => <OntologyPanel page="ontologyGraph" Component={GraphPage} />;
export const OntologyEntitiesPanel = () => <OntologyPanel page="ontologyEntities" Component={EntitiesPage} />;
export const OntologyTypesPanel = () => <OntologyPanel page="ontologyTypes" Component={EntityTypesPage} />;
export const OntologyRelationshipsPanel = () => <OntologyPanel page="ontologyRelationships" Component={RelationshipTypesPage} />;
