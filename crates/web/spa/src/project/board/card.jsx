import { EditButton } from '../../ui/permissions.jsx';
import { Button, Card, Popconfirm, Space, Tag, Typography } from 'antd';
import { MenuOutlined } from '@ant-design/icons';
import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { assignmentBadge } from '../model/board.js';
import { todoTags } from '../model/catalog.js';
export const cardId = (group, id) => `card:${JSON.stringify([group, id])}`;
export function TodoCard({ todo, group, overview, onOpen, onDelete, disabled }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: cardId(group, todo.id), data: { todoId: todo.id }, disabled });
  const badge = assignmentBadge(todo.latest_assignment);
  return <div ref={setNodeRef} style={{ transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.35 : 1 }} className="project-todo-card">
    <Card size="small" title={<Button type="link" className="project-name" onClick={() => onOpen(todo.id)}>{todo.title}</Button>}
      extra={<Button type="text" aria-label={`拖动 ${todo.title}`} icon={<MenuOutlined />} disabled={disabled} {...attributes} {...listeners} />}>
      <Space orientation="vertical" size={6} style={{ width: '100%' }}>
        {todo.draft && <Typography.Paragraph ellipsis={{ rows: 2 }} style={{ margin: 0 }}>{todo.draft}</Typography.Paragraph>}
        <Space wrap size={[0, 4]}>{todoTags(overview, todo).map((tag) => <Tag key={tag.id}>{tag.name}</Tag>)}{badge && <Tag color={badge.color}>{badge.label}</Tag>}</Space>
        <Space><Button size="small" onClick={() => onOpen(todo.id)}>详情</Button><Popconfirm title="删除该 TODO？" onConfirm={() => onDelete(todo)}><EditButton size="small" danger disabled={disabled}>删除</EditButton></Popconfirm></Space>
      </Space>
    </Card>
  </div>;
}
