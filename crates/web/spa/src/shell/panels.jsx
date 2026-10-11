import { UsersPanel } from '../admin/users.jsx';
// panels.jsx — store `page` → panel component map. Extracted from main.jsx so
// the shell contract tests can render any single page without importing the
// app shell (main.jsx auto-mounts <App/> at import time).

import { AgentsPanel } from '../agentsConfig.jsx';
import { ChatPanel } from '../chat.jsx';
import { DagPanel } from '../dagPanel.jsx';
import { FleetBrainPanel as BrainPanel } from '../fleet/brain.jsx';
import { ExecutionsPanel as TopicsPanel } from '../fleet/executions.jsx';
import { FleetNodesPanel as NodesPanel } from '../fleet/nodes.jsx';
import { FleetTeamsPanel as TeamPanel } from '../fleet/teams.jsx';
import { ProjectPanel } from '../project/project.jsx';
import { ProjectTagsPanel } from '../project/tags/panel.jsx';
import { SchedulePanel } from '../schedule/panel.jsx';
import { TodoPanel } from '../todoPanel.jsx';
import { OntologyGraphPanel, OntologyEntitiesPanel, OntologyTypesPanel, OntologyRelationshipsPanel } from '../ontology/panels.tsx';

/// Page components keyed by store `page` — one map instead of a ternary chain
/// so adding a page stays one line. Keys must equal nav.js ALL_PAGES exactly
/// (asserted by shell/headerContract.dom.test.jsx).
export const PANELS = {
  users: UsersPanel,
  chat: ChatPanel,
  team: TeamPanel,
  topics: TopicsPanel,
  schedules: SchedulePanel,
  project: ProjectPanel,
  projectTags: ProjectTagsPanel,
  dag: DagPanel,
  todos: TodoPanel,
  agents: AgentsPanel,
  nodes: NodesPanel,
  brain: BrainPanel,
  ontologyGraph: OntologyGraphPanel,
  ontologyEntities: OntologyEntitiesPanel,
  ontologyTypes: OntologyTypesPanel,
  ontologyRelationships: OntologyRelationshipsPanel,
};
