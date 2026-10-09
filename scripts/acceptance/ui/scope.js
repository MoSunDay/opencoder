const CASES = [
  { name: 'platform', script: 'platform.js', native: true, pages: ['nodes', 'users', 'tokens', 'topics', 'schedules', 'dag', 'team', 'chat', 'agents'] },
  { name: 'project', script: 'project/main.js', args: ['--workbench-only'], pages: ['project'] },
  { name: 'project-capabilities', script: 'project/capabilities/main.js', native: true, pages: ['project', 'chat', 'dag', 'brain', 'topics'] },
  { name: 'project-board', script: 'project_workbench_ui.js', pages: ['project'] },
  { name: 'dag-results', script: 'dag_results.js', native: true, pages: ['dag', 'topics'] },
  { name: 'dag-dynamic', script: 'dag_dynamic.js', native: true, pages: ['dag'] },
  { name: 'todo', script: 'todo_workbench/main.js', pages: ['todos', 'topics'] },
  { name: 'todo-initialization', script: 'todo_initialization_ui.js', pages: ['todos'] },
  { name: 'brain', cargo: true, pages: ['brain', 'topics'] },
  { name: 'ontology', python: 'ontology/main.py', pages: ['ontologyGraph', 'ontologyEntities', 'ontologyTypes', 'ontologyRelationships', 'ontologyEnvironments'] },
  { name: 'tui', terminal: true, pages: [] },
];

function verifyCoverage(pages, cases = CASES) {
  const names = cases.map((item) => item.name);
  if (new Set(names).size !== names.length) throw new Error('duplicate acceptance case');
  const covered = new Set(cases.flatMap((item) => item.pages));
  const missing = pages.filter((page) => !covered.has(page));
  const unknown = [...covered].filter((page) => !pages.includes(page));
  if (missing.length || unknown.length) throw new Error(`UI coverage mismatch: missing=${missing.join(',')} unknown=${unknown.join(',')}`);
  return Object.fromEntries(pages.map((page) => [page, cases.filter((item) => item.pages.includes(page)).map((item) => item.name)]));
}

module.exports = { CASES, verifyCoverage };
