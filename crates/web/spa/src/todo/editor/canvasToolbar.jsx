// canvasToolbar.jsx — floating controls for the TODO spec editor canvas:
// TodoCanvasToolbar is the absolute-positioned button row (auto-layout /
// fit / validation problems popover), TodoPalette is the left-hand
// draggable "add todo" panel. Both are presentational; state lives in the
// parent. Mirrors dag/editor/canvasToolbar.jsx with the single-card
// palette the todo model needs (no step kinds to choose from).

import {
  AimOutlined,
  CheckCircleOutlined,
  CheckSquareOutlined,
  NodeIndexOutlined,
} from '@ant-design/icons';
import { Badge, Button, Popover, Space, Typography } from 'antd';

const { Text } = Typography;

/// TodoCanvasToolbar — renders .dag-edit-toolbar (positioned by app.css
/// over the canvas). problems is the specLevelProblems() message list for
/// the current draft (spec-level issues that cannot be pinned on a node):
/// any entry turns the 校验 button danger with a Badge count and a
/// clickable problem list; an empty list shows the passing hint instead.
export function TodoCanvasToolbar({ problems, onAutoLayout, onFitView }) {
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

/// TodoPalette — vertical add-todo panel. The single card is an HTML5 drag
/// source (dataTransfer 'application/opencoder-todo' carries 'todo' for the
/// canvas drop handler) and also plain-clickable via onAdd().
export function TodoPalette({ onAdd }) {
  return (
    <div className="dag-edit-pal">
      <Text type="secondary">添加 TODO</Text>
      <div
        className="dag-edit-pal-card"
        draggable
        onDragStart={(e) => {
          e.dataTransfer.setData('application/opencoder-todo', 'todo');
          e.dataTransfer.effectAllowed = 'move';
        }}
        onClick={() => onAdd()}
      >
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, fontSize: 13 }}>
          <CheckSquareOutlined />
          TODO 节点
        </span>
        <Text type="secondary" style={{ fontSize: 11 }}>
          独立执行的待办
        </Text>
      </div>
    </div>
  );
}
