// Pure catalog projections; one optional project → initiative → TODO hierarchy.
export function flattenInitiatives(overview) {
  return [
    ...(overview?.goals || []).flatMap((goal) => (goal.initiatives || []).map((item) => ({ ...item, goal_id: goal.id, goal_title: goal.title }))),
    ...(overview?.standalone_initiatives || []).map((item) => ({ ...item, goal_id: null, goal_title: null })),
  ];
}
export function flattenTodos(overview) {
  return [
    ...flattenInitiatives(overview).flatMap((group) => (group.todos || []).map((todo) => ({ ...todo, initiative_id: group.id, group_title: group.title, goal_id: group.goal_id, goal_title: group.goal_title }))),
    ...(overview?.backlog || []).map((todo) => ({ ...todo, initiative_id: null, group_title: null, goal_id: null, goal_title: null })),
  ];
}
export const projectOptions = (overview) => (overview?.goals || []).map((g) => ({ value: g.id, label: g.title }));
export const initiativeOptions = (overview) => flattenInitiatives(overview).map((i) => ({ value: i.id, label: `${i.title} · ${i.goal_title || '独立专项'}` }));
export const groupOptions = initiativeOptions;
export const matchesText = (query, ...values) => values.some((v) => String(v || '').toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
export const searchSelect = { showSearch: true, optionFilterProp: 'label', allowClear: true };
