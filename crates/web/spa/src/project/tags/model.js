import { flattenInitiatives } from '../model/relations.js';

export function tagRows(overview) {
  const projects = new Map((overview?.goals || []).map((row) => [row.id, row]));
  const initiatives = new Map(flattenInitiatives(overview).map((row) => [row.id, row]));
  return (overview?.tags || []).map((tag) => {
    const initiative = tag.scope_type === 'initiative' ? initiatives.get(tag.scope_id) : null;
    return {
      ...tag,
      scope_label: tag.scope_type === 'initiative' ? '专项' : '项目',
      project_title: tag.scope_type === 'project'
        ? projects.get(tag.scope_id)?.title || '项目已删除'
        : initiative?.goal_title || (initiative ? '独立专项' : '专项已删除'),
      initiative_title: tag.scope_type === 'initiative' ? initiative?.title || '专项已删除' : '—',
    };
  }).sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
}
