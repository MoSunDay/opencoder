import { EditButton, useCanEdit } from '../../ui/permissions.jsx';
import { Button, Card, Collapse, Input, Segmented, Select, Space, Typography } from 'antd';
import { DndContext, DragOverlay, KeyboardSensor, PointerSensor, closestCorners, pointerWithin, useDroppable, useSensor, useSensors } from '@dnd-kit/core';
import { SortableContext, sortableKeyboardCoordinates, verticalListSortingStrategy } from '@dnd-kit/sortable';
import { useRef, useState } from 'react';
import { apiDel, apiPut } from '../../api.js';
import { err, ok } from '../../notice.js';
import { LANES, laneOf, moveTodo, ordered } from '../model/board.js';
import { effectiveTags, resolveTagFilter, tagGroups } from '../model/catalog.js';
import { matchesText } from '../model/relations.js';
import { CreateTodo } from '../views/todoForm.jsx';
import { cardId, TodoCard } from './card.jsx';
import { useProjectView } from '../views/viewState.jsx';

function boardCollision(args) {
  const hits = pointerWithin(args);
  const cards = hits.filter((hit) => hit.data?.droppableContainer.data.current?.todoId);
  return cards.length ? cards : hits.length ? hits : closestCorners(args);
}
function Lane({ group, status, title, rows, ...props }) {
  const { setNodeRef, isOver } = useDroppable({ id: `lane:${JSON.stringify([group, status])}`, data: { status } });
  return <section ref={setNodeRef} aria-label={`${props.groupTitle || ''} ${title}`.trim()} className={`project-board-lane${isOver ? ' project-board-lane-over' : ''}`}>
    <Typography.Text strong>{title} · {rows.length}</Typography.Text>
    <SortableContext items={rows.map((todo) => cardId(group, todo.id))} strategy={verticalListSortingStrategy}>
      {rows.map((todo) => <TodoCard key={todo.id} todo={todo} group={group} {...props} />)}
    </SortableContext>
  </section>;
}
export function TodoBoard({ initiative, overview, refresh, onNotice, openTodo }) {
  const canEdit = useCanEdit();
  const [view, setView] = useProjectView(`board:${initiative?.id}`, { query: '', tagFilter: [], grouped: false });
  const { query, tagFilter, grouped } = view;
  const setQuery = (query) => setView((v) => ({ ...v, query }));
  const setTagFilter = (tagFilter) => setView((v) => ({ ...v, tagFilter }));
  const setGrouped = (grouped) => setView((v) => ({ ...v, grouped }));
  const [creating, setCreating] = useState(false);
  const [preview, setPreview] = useState(null);
  const [activeTodo, setActiveTodo] = useState(null);
  const [busy, setBusy] = useState(false);
  const saving = useRef(false);
  const all = preview || (initiative?.todos || []).map((todo) => ({ ...todo, initiative_id: initiative.id }));
  const tags = effectiveTags(overview, initiative?.id);
  const activeTagIds = resolveTagFilter(tags, tagFilter);
  const shown = all.filter((todo) => matchesText(query, todo.title, todo.draft) && (!activeTagIds.length || activeTagIds.some((id) => (todo.tag_ids || []).includes(id))));
  const groups = tagGroups(shown, tags.filter((tag) => !activeTagIds.length || activeTagIds.includes(tag.id)), grouped);
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }), useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }));
  const remove = async (todo) => {
    try { await apiDel(`/api/project/todos/${encodeURIComponent(todo.id)}`); await refresh(); onNotice(ok('TODO 已删除')); }
    catch (error) { onNotice(err(error.message)); }
  };
  const move = async ({ active, over }) => {
    setActiveTodo(null);
    if (!canEdit || !over || saving.current) return;
    const activeId = active.data.current?.todoId;
    const overId = over.data.current?.todoId || `lane:${over.data.current?.status}`;
    if (activeId === overId) return;
    const result = moveTodo(all, activeId, overId);
    if (!result) return;
    saving.current = true; setBusy(true); setPreview(result.preview);
    try {
      await apiPut('/api/project/todos/order', { initiative_id: initiative.id, board_status: result.status, ids: result.ids });
      await refresh();
    } catch (error) { onNotice(err(`状态保存失败，已恢复原状态：${error.message}`)); }
    finally { setPreview(null); setBusy(false); saving.current = false; }
  };
  const board = (group) => <div className="project-board-columns">{LANES.map(([status, title]) => <Lane key={status} group={group.id} groupTitle={group.title} status={status} title={title}
    rows={ordered(group.rows.filter((todo) => laneOf(todo) === status))} overview={overview} onOpen={openTodo} onDelete={remove} disabled={!canEdit || busy} />)}</div>;
  return <Space orientation="vertical" size={12} style={{ width: '100%' }}>
    <Space wrap><EditButton type="primary" onClick={() => setCreating(true)}>新建 TODO</EditButton>
      <Input.Search aria-label="搜索专项 TODO" placeholder="搜索标题或说明" value={query} onChange={(e) => setQuery(e.target.value)} allowClear style={{ width: 220 }} />
      <Select mode="multiple" showSearch optionFilterProp="label" aria-label="筛选 Tag" placeholder="全部 Tag" value={activeTagIds} onChange={(ids) => setTagFilter(tags.filter((tag) => ids.includes(tag.id)).map(({ id, name }) => ({ id, name })))} allowClear style={{ minWidth: 180, maxWidth: '100%' }} options={tags.map((tag) => ({ label: tag.name, value: tag.id }))} />
      <Segmented aria-label="卡片分组" value={grouped} onChange={setGrouped} options={[{ label: '不分组', value: false }, { label: '按 Tag 分组', value: true }]} />
    </Space>
    {busy && <Typography.Text type="secondary">正在保存状态和顺序…</Typography.Text>}
    <DndContext sensors={sensors} collisionDetection={boardCollision} onDragStart={({ active }) => setActiveTodo(all.find((todo) => todo.id === active.data.current?.todoId))} onDragCancel={() => setActiveTodo(null)} onDragEnd={move}>
      {grouped ? <Collapse defaultActiveKey={groups.map((group) => group.id)} items={groups.map((group) => ({ key: group.id, label: `${group.title} · ${group.rows.length}`, children: board(group) }))} /> : groups.map((group) => <div key={group.id}>{board(group)}</div>)}
      <DragOverlay>{activeTodo && <Card size="small">{activeTodo.title}</Card>}</DragOverlay>
    </DndContext>
    <CreateTodo key={creating ? initiative?.id : 'closed'} open={creating} overview={overview} initiativeId={initiative?.id} onNotice={onNotice} onClose={() => setCreating(false)}
      onCreated={async (id) => { setCreating(false); await refresh(); openTodo(id); }} />
  </Space>;
}
