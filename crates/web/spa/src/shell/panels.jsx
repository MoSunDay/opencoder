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
import { OwnerViewPanel } from '../project/ownerViewPanel.jsx';
import { ProgressPanel } from '../project/progressPanel.jsx';
import { ProjectPanel } from '../project/project.jsx';
import { SchedulePanel } from '../schedule/panel.jsx';
import { TodoPanel } from '../todoPanel.jsx';

/// Page components keyed by store `page` — one map instead of a ternary chain
/// so adding a page stays one line. Keys must equal nav.js ALL_PAGES exactly
/// (asserted by shell/headerContract.dom.test.jsx).
export const PANELS = {
  chat: ChatPanel,
  team: TeamPanel,
  topics: TopicsPanel,
  schedules: SchedulePanel,
  project: ProjectPanel,
  progress: ProgressPanel,
  ownerview: OwnerViewPanel,
  dag: DagPanel,
  todos: TodoPanel,
  agents: AgentsPanel,
  nodes: NodesPanel,
  brain: BrainPanel,
};
