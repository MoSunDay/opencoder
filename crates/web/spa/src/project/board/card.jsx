import { Avatar, Button, Card, Dropdown, Modal, Tag } from 'antd';
import { LinkOutlined, MoreOutlined, UserOutlined, DeleteOutlined } from '@ant-design/icons';
import { useMemo } from 'react';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { assignmentBadge, laneOf } from '../model/board.js';
import { todoTags } from '../model/catalog.js';
import { taskSummary } from './summary.js';
import './card.css';
export const cardId = (group, id) => `card:${JSON.stringify([group, id])}`;
export function TodoCard({ todo, group, overview, onOpen, onDelete, disabled }) {
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition, isDragging } = useSortable({ id: cardId(group, todo.id), data: { todoId: todo.id }, disabled });
  const badge = assignmentBadge(todo.latest_assignment);
  const [modal, holder] = Modal.useModal();
  const summary = useMemo(() => taskSummary(todo.draft), [todo.draft]);
  return <div ref={setNodeRef} style={{ transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.35 : 1 }} className="project-todo-card">
    {holder}
    <Card size="small" className={`project-task-card project-task-card--${laneOf(todo)}${disabled ? '' : ' project-task-card--draggable'}`}
      onMouseDown={listeners?.onMouseDown} onTouchStart={listeners?.onTouchStart}>
      <div className="project-card-heading">
        <button ref={setActivatorNodeRef} type="button" className="project-card-title" {...(disabled ? {} : attributes)}
          onKeyDown={(event) => { if (event.code === 'Space') listeners?.onKeyDown?.(event); }}
          onClick={() => onOpen(todo.id)} title={todo.title}>{todo.title}</button>
        <div className="project-card-controls" onMouseDown={(event) => event.stopPropagation()} onTouchStart={(event) => event.stopPropagation()}>
          <Dropdown trigger={['click']} menu={{ items: [{ key: 'delete', label: '删除任务', icon: <DeleteOutlined />, danger: true, disabled }],
            onClick: () => { if (!disabled) modal.confirm({ title: '删除该 TODO？', content: todo.title, okText: '删除', cancelText: '取消', okButtonProps: { danger: true }, onOk: () => onDelete(todo) }); } }}>
            <Button type="text" size="small" aria-label={`更多操作 ${todo.title}`} icon={<MoreOutlined />} disabled={disabled} />
          </Dropdown>
        </div>
      </div>
      {summary && <p className="project-card-summary">{summary}</p>}
      {!!todoTags(overview, todo).length && <div className="project-card-tags">{todoTags(overview, todo).map((tag) => <Tag key={tag.id}>{tag.name}</Tag>)}</div>}
      <div className="project-card-footer">
        <span className="project-card-owner" title={`负责人：${todo.owner_user_name || '未指定'}`}>
          <Avatar size={22} icon={!todo.owner_user_name && <UserOutlined />}>{todo.owner_user_name && Array.from(todo.owner_user_name)[0].toUpperCase()}</Avatar>
          <span>{todo.owner_user_name || '未指定负责人'}</span>
        </span>
        {badge && <span className="project-card-assignment"><LinkOutlined />{badge.label}</span>}
      </div>
    </Card>
  </div>;
}
