import { expect, it } from 'vitest';
import { projectHierarchy } from './hierarchy.js';

const overview = {
  goals: [{ id: 'p', initiatives: [{ id: 'i', todos: [{ id: 't' }] }] }],
  standalone_initiatives: [{ id: 'solo', todos: [{ id: 'solo-t' }] }],
  backlog: [{ id: 'free' }],
};

it('restores the real ancestors for ID-only links', () => {
  expect(projectHierarchy(overview, { todoId: 't' })).toEqual({ projectId: 'p', initiativeId: 'i', todoId: 't' });
  expect(projectHierarchy(overview, { initiativeId: 'i' })).toEqual({ projectId: 'p', initiativeId: 'i', todoId: null });
});

it('uses current ownership when shared ancestor parameters are stale', () => {
  expect(projectHierarchy(overview, { projectId: 'old-p', initiativeId: 'old-i', todoId: 't' }))
    .toEqual({ projectId: 'p', initiativeId: 'i', todoId: 't' });
  expect(projectHierarchy(overview, { projectId: 'old-p', initiativeId: 'i' }))
    .toEqual({ projectId: 'p', initiativeId: 'i', todoId: null });
});

it('stops at a standalone initiative or an unassigned TODO', () => {
  expect(projectHierarchy(overview, { projectId: 'old-p', todoId: 'solo-t' }))
    .toEqual({ projectId: null, initiativeId: 'solo', todoId: 'solo-t' });
  expect(projectHierarchy(overview, { projectId: 'old-p', initiativeId: 'solo' }))
    .toEqual({ projectId: null, initiativeId: 'solo', todoId: null });
  expect(projectHierarchy(overview, { projectId: 'old-p', initiativeId: 'old-i', todoId: 'free' }))
    .toEqual({ projectId: null, initiativeId: null, todoId: 'free' });
});

it('preserves unresolved links during loading, errors and missing records', () => {
  const requested = { projectId: 'p', initiativeId: 'i', todoId: 'missing' };
  expect(projectHierarchy(null, requested)).toEqual(requested);
  expect(projectHierarchy(overview, requested)).toEqual(requested);
  expect(projectHierarchy(overview, null)).toEqual({ projectId: null, initiativeId: null, todoId: null });
});

it('follows updated ownership without changing the selected TODO', () => {
  const moved = { ...overview, goals: [{ id: 'next-p', initiatives: [{ id: 'next-i', todos: [{ id: 't' }] }] }] };
  expect(projectHierarchy(moved, projectHierarchy(overview, { todoId: 't' })))
    .toEqual({ projectId: 'next-p', initiativeId: 'next-i', todoId: 't' });
});
