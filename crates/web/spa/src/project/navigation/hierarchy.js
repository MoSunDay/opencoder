import { flattenInitiatives, flattenTodos } from '../model/relations.js';

// Resolve parents from the current catalog, including ID-only shared links.
// Keep unresolved destinations intact so loading and error views remain usable.
export function projectHierarchy(overview, selection) {
  const requested = { projectId: null, initiativeId: null, todoId: null, ...selection };
  if (!overview) return requested;
  if (requested.todoId) {
    const todo = flattenTodos(overview).find((item) => item.id === requested.todoId);
    if (todo) return { projectId: todo.goal_id, initiativeId: todo.initiative_id, todoId: todo.id };
  }
  if (requested.initiativeId) {
    const initiative = flattenInitiatives(overview).find((item) => item.id === requested.initiativeId);
    if (initiative) return { ...requested, projectId: initiative.goal_id };
  }
  return requested;
}
