import { expect, it } from 'vitest';
import { changePresentation, committedPositions, displayNodes } from './presentation.js';
const node = { id: 'layer:a', type: 'layer', position: { x: 10, y: 20 }, width: 420, height: 290, data: { title: '旧任务' } };

it('a plan redraw preserves an active drag and measurements while showing the latest task', () => {
  const current = changePresentation({}, [
    { type: 'dimensions', id: node.id, dimensions: { width: 420, height: 290 } },
    { type: 'position', id: node.id, position: { x: 100, y: 200 }, dragging: true },
  ], [node], true);
  const [drawn] = displayNodes([{ ...node, data: { title: '最新任务' } }], current);
  expect(drawn.position).toEqual({ x: 100, y: 200 });
  expect(drawn.measured).toEqual({ width: 420, height: 290 });
  expect(drawn.data.title).toBe('最新任务');
  expect(displayNodes([], current)).toEqual([]);
  expect(displayNodes([{ ...node, width: 648 }], current)[0].measured).toBeUndefined();
});

it('drag completion and keyboard movement commit positions but selection and measurements do not', () => {
  expect(committedPositions([{ type: 'position', id: node.id, position: { x: 11, y: 22 }, dragging: true }], [node])).toEqual({});
  const finish = [{ type: 'position', id: node.id, position: { x: 40, y: 60 }, dragging: false }];
  expect(committedPositions(finish, [node])).toEqual({ a: { x: 40, y: 60 } });
  const state = changePresentation({ [node.id]: { dragging: true, position: { x: 30, y: 50 } } }, finish, [node], true);
  expect(displayNodes([{ ...node, position: { x: 40, y: 60 } }], state)[0].position).toEqual({ x: 40, y: 60 });
  expect(committedPositions([{ type: 'position', id: node.id, position: { x: 41, y: 60 } }], [node])).toEqual({ a: { x: 41, y: 60 } });
  expect(committedPositions([{ type: 'select', id: node.id, selected: true }], [node])).toEqual({});
});

it('display events never change plan membership and readonly position events are ignored', () => {
  const state = changePresentation({}, [{ type: 'remove', id: node.id }, { type: 'position', id: node.id, position: { x: 99, y: 99 }, dragging: true }], [node], false);
  expect(displayNodes([node], state)).toMatchObject([{ id: node.id, position: { x: 10, y: 20 } }]);
});
