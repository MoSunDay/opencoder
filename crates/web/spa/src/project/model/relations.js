// Pure catalog projections shared by lists, selectors, drawers and rollups.
export function flattenMilestones(overview) {
  return [
    ...(overview?.goals || []).flatMap((goal) => (goal.milestones || []).map((m) => ({
      ...m, goal_id: goal.id, goal_title: goal.title,
    }))),
    ...(overview?.standalone_milestones || []).map((m) => ({ ...m, goal_id: null, goal_title: null })),
  ];
}

export function flattenTodos(overview) {
  return [
    ...flattenMilestones(overview).flatMap((m) => (m.todos || []).map((todo) => ({
      ...todo, milestone_id: m.id, milestone_title: m.title, goal_id: m.goal_id, goal_title: m.goal_title,
    }))),
    ...(overview?.backlog || []).map((todo) => ({ ...todo, milestone_id: null, milestone_title: null, goal_id: null, goal_title: null })),
  ];
}

export const goalOptions = (overview) => (overview?.goals || []).map((g) => ({
  value: g.id, label: `${g.title} · ${g.id}`,
}));

export const milestoneOptions = (overview) => flattenMilestones(overview).map((m) => ({
  value: m.id, label: `${m.title} · ${m.goal_title || '独立专项'} · ${m.id}`,
}));

export const matchesText = (query, ...values) => values.some((value) =>
  String(value || '').toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));

export const searchSelect = { showSearch: true, optionFilterProp: 'label', allowClear: true };
