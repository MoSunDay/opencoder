// todoNode.jsx — React Flow node card for the TODO spec editor canvas.
// Presentational only: a line-by-line mirror of dag/editor/stepNode.jsx
// (Left target / Right source handles, fixed box mirrored by
// canvasLayout.js TODO_NODE_W/H) rendering the EDIT payload — todo title,
// agent badge, a one-line dep summary and an invalid dot so validateSpec
// problems can be flagged right on the node. data: { todo, depNames,
// invalid }.

import { CheckSquareOutlined } from '@ant-design/icons';
import { Handle, Position } from '@xyflow/react';

/// TodoEditNode — one editable todo card. Kept module-level and stable via
/// editNodeTypes so React Flow does not remount nodes on parent re-renders.
export function TodoEditNode({ data, selected }) {
  const todo = (data && data.todo) || {};
  const invalid = !!(data && data.invalid);
  const depNames = Array.isArray(data && data.depNames) ? data.depNames : [];
  // An untitled todo shows its id — the card must stay identifiable while
  // validateSpec still flags the empty title.
  const title =
    typeof todo.title === 'string' && todo.title.trim() ? todo.title : todo.id;
  const cls =
    'dag-edit-node dag-edit-node--todo' +
    (invalid ? ' dag-edit-node--invalid' : '') +
    (selected ? ' dag-edit-node--selected' : '');
  return (
    <div className={cls}>
      <Handle type="target" position={Position.Left} isConnectable={true} />
      <div className="dag-edit-node-head">
        {invalid ? <span className="dag-edit-node-dot" title="校验未通过" /> : null}
        <CheckSquareOutlined />
        <span className="dag-edit-node-name">{title}</span>
        <span className="dag-edit-node-kind">{todo.agent || '-'}</span>
      </div>
      <div className="dag-edit-node-deps">
        {depNames.length ? '依赖: ' + depNames.join(', ') : '无依赖'}
      </div>
      <Handle type="source" position={Position.Right} isConnectable={true} />
    </div>
  );
}

/// editNodeTypes — stable nodeTypes map handed to the editor ReactFlow.
export const editNodeTypes = { todoEdit: TodoEditNode };
