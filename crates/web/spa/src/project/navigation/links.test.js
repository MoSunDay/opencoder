import { expect, it } from 'vitest';
import { projectLink, projectLocation, projectTab } from './links.js';
import { linkedPage } from '../../fleet/navigation/links.js';

it('round trips encoded IDs, nested drawers and embedded paths', () => {
  const selected = { projectId: '项目 / #&?', initiativeId: '子项+1', todoId: '任务=2' };
  const href = projectLocation('https://example.test/console/?keep=yes#note=keep', selected);
  const url = new URL(href, 'https://example.test');
  expect(url.pathname).toBe('/console/');
  expect(url.search).toBe('?keep=yes');
  expect(projectLink(url.search, url.hash)).toEqual(selected);
  expect(url.hash).toContain('note=keep');
  expect(linkedPage(url.search, url.hash)).toBe('project');
  expect(projectTab(selected)).toBe('projects');
});

it('opens ID-only links and gives fragments precedence over query parameters', () => {
  expect(projectLink('?todo=old', '#initiative=current')).toEqual({ projectId: null, initiativeId: 'current', todoId: null });
  expect(projectLink('?todo=task')).toEqual({ projectId: null, initiativeId: null, todoId: 'task' });
  expect(projectLink('?project=+&todo=')).toBeNull();
  expect(projectTab(projectLink('', '#todo=t'))).toBe('todos');
  expect(projectTab(projectLink('', '#initiative=i'))).toBe('initiatives');
});

it('clears only project state on close and removes conflicting routes when opening', () => {
  expect(projectLocation('https://test/console/?keep=1&todo=old#project=p&note=2')).toBe('/console/?keep=1#note=2');
  expect(projectLocation('https://test/?todo=old#section')).toBe('/#section');
  const next = new URL(projectLocation('https://test/?execution=e&brain_run=b#execution_kind=dag&todo=old', { todoId: 'next' }), 'https://test');
  expect(next.search).toBe('');
  expect(next.hash).toBe('#todo=next');
  expect(linkedPage(next.search, next.hash)).toBe('project');
  expect(linkedPage('?todo=t', '#execution=e')).toBe('topics');
});
