// Pure catalog projections shared by lists, selectors, drawers and rollups.
const flattenGroup = (overview, field, standalone) => [
  ...(overview?.goals || []).flatMap((goal) => (goal[field] || []).map((item) => ({
    ...item, goal_id: goal.id, goal_title: goal.title,
  }))),
  ...(overview?.[standalone] || []).map((item) => ({ ...item, goal_id: null, goal_title: null })),
];

export function flattenMilestones(overview) {
  return flattenGroup(overview, 'milestones', 'standalone_milestones');
}

export function flattenInitiatives(overview) {
  return flattenGroup(overview, 'initiatives', 'standalone_initiatives');
}

export const allGroups = (overview) => [
  ...flattenMilestones(overview).map((item) => ({ ...item, group_type: 'milestone' })),
  ...flattenInitiatives(overview).map((item) => ({ ...item, group_type: 'initiative' })),
];

export function flattenTodos(overview) {
  return [
    ...allGroups(overview).flatMap((group) => (group.todos || []).map((todo) => ({
      ...todo, milestone_id: group.id, group_title: group.title, group_type: group.group_type, goal_id: group.goal_id, goal_title: group.goal_title,
    }))),
    ...(overview?.backlog || []).map((todo) => ({ ...todo, milestone_id: null, milestone_title: null, goal_id: null, goal_title: null })),
  ];
}

export const projectOptions = (overview) => (overview?.goals || []).map((g) => ({
  value: g.id, label: `${g.title} · ${g.id}`,
}));

export const initiativeOptions = (overview) => flattenInitiatives(overview).map((m) => ({
  value: m.id, label: `专项 · ${m.title} · ${m.goal_title || '未关联项目'} · ${m.id}`,
}));

export const groupOptions = (overview) => [
  ...flattenMilestones(overview).map((item) => ({ value: item.id, label: `里程碑 · ${item.title} · ${item.goal_title || '未关联项目'} · ${item.id}` })),
  ...initiativeOptions(overview),
];

export const matchesText = (query, ...values) => values.some((value) =>
  String(value || '').toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));

export const searchSelect = { showSearch: true, optionFilterProp: 'label', allowClear: true };
