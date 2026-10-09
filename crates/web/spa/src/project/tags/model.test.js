import { expect, it } from 'vitest';
import { tagRows } from './model.js';

it('keeps same-name definitions separate and resolves project and standalone owners', () => {
  const overview = {
    goals: [{ id: 'p', title: '项目甲', initiatives: [{ id: 'i', title: '专项甲' }] }],
    standalone_initiatives: [{ id: 's', title: '独立专项甲' }],
    tags: [
      { id: 'parent', name: '模块', scope_type: 'project', scope_id: 'p' },
      { id: 'local', name: '模块', scope_type: 'initiative', scope_id: 'i' },
      { id: 'solo', name: '独立', scope_type: 'initiative', scope_id: 's' },
    ],
  };
  const rows = tagRows(overview);
  expect(rows).toHaveLength(3);
  expect(rows.find((row) => row.id === 'parent')).toMatchObject({ scope_label: '项目', project_title: '项目甲', initiative_title: '—' });
  expect(rows.find((row) => row.id === 'local')).toMatchObject({ scope_label: '专项', project_title: '项目甲', initiative_title: '专项甲' });
  expect(rows.find((row) => row.id === 'solo')).toMatchObject({ project_title: '独立专项', initiative_title: '独立专项甲' });
  expect(overview.tags[0]).not.toHaveProperty('project_title');
});

it('handles empty data and keeps orphan definitions visible', () => {
  expect(tagRows(null)).toEqual([]);
  expect(tagRows({ tags: [{ id: 'x', name: '失效', scope_type: 'initiative', scope_id: 'gone' }] })).toEqual([
    expect.objectContaining({ id: 'x', project_title: '专项已删除', initiative_title: '专项已删除' }),
  ]);
});
