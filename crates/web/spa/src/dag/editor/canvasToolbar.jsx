// canvasToolbar.jsx — floating controls for the DAG spec editor canvas:
// CanvasToolbar is the absolute-positioned button row (auto-layout / fit /
// validation problems popover), StepPalette is the left-hand draggable
// "add step" panel. Both are presentational; state lives in the parent.

import {
  AimOutlined,
  CheckCircleOutlined,
  CodeOutlined,
  NodeIndexOutlined,
  RobotOutlined,
} from '@ant-design/icons';
import { Badge, Button, Popover, Space, Typography } from 'antd';

const { Text } = Typography;

/// CanvasToolbar — renders .dag-edit-toolbar (positioned by app.css over the
/// canvas). problems is the validateSpec() string list for the current
/// draft: any entry turns the 校验 button danger with a Badge count and a
/// clickable problem list; an empty list shows the passing hint instead.
export function CanvasToolbar({ onAutoLayout, onFitView, problems }) {
  const list = Array.isArray(problems) ? problems : [];
  const hasProblems = list.length > 0;
  const content = hasProblems ? (
    <ul style={{ margin: 0, paddingLeft: 18, maxHeight: 240, overflowY: 'auto' }}>
      {list.map((p, i) => (
        <li key={i}>{p}</li>
      ))}
    </ul>
  ) : (
    '校验通过，可保存'
  );
  return (
    <div className="dag-edit-toolbar">
      <Space.Compact size="small">
        <Button size="small" icon={<NodeIndexOutlined />} onClick={onAutoLayout}>
          自动布局
        </Button>
        <Button size="small" icon={<AimOutlined />} onClick={onFitView}>
          适应画布
        </Button>
        <Popover title={hasProblems ? '校验问题' : undefined} content={content} trigger="click">
          <Badge count={list.length} size="small">
            <Button size="small" danger={hasProblems} icon={hasProblems ? null : <CheckCircleOutlined />}>
              校验
            </Button>
          </Badge>
        </Popover>
      </Space.Compact>
    </div>
  );
}

const PALETTE = [
  { kindType: 'agent', icon: <RobotOutlined />, title: 'Agent 步骤', hint: 'LLM 提示词执行' },
  { kindType: 'wasm', icon: <CodeOutlined />, title: 'Wasm 步骤', hint: '内嵌 wasm runtime / runc 沙箱' },
];

/// StepPalette — vertical add-step panel. Each card is an HTML5 drag source
/// (dataTransfer 'application/opencoder-step' carries the kind type for the
/// canvas drop handler) and also plain-clickable via onAdd(kindType).
export function StepPalette({ onAdd }) {
  return (
    <div className="dag-edit-pal">
      <Text type="secondary">添加步骤</Text>
      {PALETTE.map((c) => (
        <div
          key={c.kindType}
          className="dag-edit-pal-card"
          draggable
          onDragStart={(e) => {
            e.dataTransfer.setData('application/opencoder-step', c.kindType);
            e.dataTransfer.effectAllowed = 'move';
          }}
          onClick={() => onAdd(c.kindType)}
        >
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, fontSize: 13 }}>
            {c.icon}
            {c.title}
          </span>
          <Text type="secondary" style={{ fontSize: 11 }}>
            {c.hint}
          </Text>
        </div>
      ))}
    </div>
  );
}
