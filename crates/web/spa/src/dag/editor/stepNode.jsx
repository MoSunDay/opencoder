// stepNode.jsx — React Flow node card for the DAG spec editor canvas.
// Presentational only: follows the runDetail.jsx dagStepNode pattern
// (Left target / Right source handles, fixed box mirrored by canvasLayout.js
// EDIT_NODE_W/H) but renders the EDIT payload — step name, kind badge, a
// one-line dep summary and an invalid dot so validateSpec problems can be
// flagged right on the node. data: { step, kindType, depNames, invalid }.

import { CodeOutlined, RobotOutlined } from '@ant-design/icons';
import { Handle, Position } from '@xyflow/react';

const KIND_LABEL = { agent: 'Agent', wasm: 'Wasm', runner: 'Runner' };

/// StepEditNode — one editable step card. Kept module-level and stable via
/// editNodeTypes so React Flow does not remount nodes on parent re-renders.
export function StepEditNode({ data, selected }) {
  const step = (data && data.step) || {};
  const kindType = (data && data.kindType) || '';
  const invalid = !!(data && data.invalid);
  const depNames = Array.isArray(data && data.depNames) ? data.depNames : [];
  const icon =
    kindType === 'wasm' ? <CodeOutlined /> : kindType === 'agent' ? <RobotOutlined /> : null;
  const cls =
    'dag-edit-node dag-edit-node--' +
    kindType +
    (invalid ? ' dag-edit-node--invalid' : '') +
    (selected ? ' dag-edit-node--selected' : '');
  return (
    <div className={cls}>
      <Handle type="target" position={Position.Left} isConnectable={true} />
      <div className="dag-edit-node-head">
        {invalid ? <span className="dag-edit-node-dot" title="校验未通过" /> : null}
        {icon}
        <span className="dag-edit-node-name">{step.name}</span>
        <span className="dag-edit-node-kind">{KIND_LABEL[kindType] || kindType || '-'}</span>
      </div>
      <div className="dag-edit-node-deps">
        {depNames.length ? '依赖: ' + depNames.join(', ') : '无依赖'}
      </div>
      <Handle type="source" position={Position.Right} isConnectable={true} />
    </div>
  );
}

/// editNodeTypes — stable nodeTypes map handed to the editor ReactFlow.
export const editNodeTypes = { stepEdit: StepEditNode };
