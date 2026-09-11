import { Alert, Button, Descriptions, Space, Spin, Tag, Typography } from 'antd';
import { useState } from 'react';
import { apiGet } from '../../api.js';
import { Markdown } from '../../project/markdown.jsx';
import { InlineFields, PayloadWindows } from './fields.jsx';

function PageButtons({ page, busy, more, previous, next }) {
  return <Space style={{ marginTop: 8 }}>
    <Button size="small" disabled={busy || page === 0} onClick={previous}>上一页</Button>
    <Typography.Text type="secondary">第 {page + 1} 页</Typography.Text>
    <Button size="small" disabled={busy || !more} onClick={next}>下一页</Button>
    {busy ? <Spin size="small" /> : null}
  </Space>;
}

function WindowedRows({ initialRows, initialNext, path, render }) {
  const [rows, setRows] = useState(initialRows || []);
  const [nextCursor, setNextCursor] = useState(initialNext ?? null);
  const [cursors, setCursors] = useState([null]);
  const [page, setPage] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const load = async (cursor, targetPage) => {
    if (targetPage === 0 && cursor === null) {
      setRows(initialRows || []); setNextCursor(initialNext ?? null); setPage(0); setError('');
      return;
    }
    setBusy(true); setError('');
    try {
      const result = await path(cursor);
      setRows(result.rows || []); setNextCursor(result.next ?? null); setPage(targetPage);
    } catch (e) {
      setError(e?.message || '读取下一页失败');
    } finally {
      setBusy(false);
    }
  };
  const next = () => {
    if (nextCursor === null || nextCursor === undefined) return;
    const stack = cursors.slice(0, page + 1).concat(nextCursor);
    setCursors(stack); load(nextCursor, page + 1);
  };
  const previous = () => {
    if (page === 0) return;
    load(cursors[page - 1], page - 1);
  };
  return <div>
    {error && <Alert type="error" showIcon title={error} />}
    {render(rows)}
    <PageButtons page={page} busy={busy} more={nextCursor !== null && nextCursor !== undefined} previous={previous} next={next} />
  </div>;
}

function TeamDetail({ id, detail }) {
  const team = detail?.definition || {};
  const topic = detail?.topic || {};
  const turns = Array.isArray(topic.turns) ? topic.turns : [];
  return <div><Typography.Title level={5}>团队执行</Typography.Title><Descriptions size="small" items={[
    { key: 'name', label: '团队', children: team.name || '—' },
    { key: 'captain', label: '队长', children: team.captain || '—' },
    { key: 'members', label: '成员', children: (team.members || []).map((member) => <Tag key={member.agent} color={member.agent === team.captain ? 'gold' : undefined}>{[member.agent, ...(member.capabilities || [])].join(' · ')}</Tag>) },
  ]} />
  {turns.length || topic.turns_page?.more ? <WindowedRows
    initialRows={turns}
    initialNext={topic.turns_page?.next_turn}
    path={asyncPath(`/api/executions/${encodeURIComponent(id)}/team-turns?after_turn=`, 'turns', 'next_turn')}
    render={(rows) => <Space orientation="vertical" style={{ width: '100%' }}>{rows.map((turn, index) => <TeamTurn key={turn.turn ?? index} id={id} turn={turn} />)}</Space>}
  /> : null}</div>;
}

function TeamTurn({ id, turn }) {
  const meta = turn.meta?.omitted ? {} : (turn.meta || turn);
  const number = turn.turn ?? meta.turn;
  const participants = Array.isArray(meta.participants) ? meta.participants : [];
  const subTurns = Number.isInteger(meta.sub_turns) ? meta.sub_turns : 0;
  const field = (value) => ({ omitted: true, field: value, read_via: 'detail_field' });
  const plan = turn.detail_fields?.plan || `team.turn.${number}.plan`;
  return <div className="execution-team-turn">
    <Space wrap><Typography.Text strong>第 {number ?? '?'} 轮</Typography.Text>
      <Typography.Text>{meta.question || '协作记录'}</Typography.Text>
      {participants.map((member) => <Tag key={member}>{member}</Tag>)}
      {typeof meta.aligned === 'boolean' ? <Tag color={meta.aligned ? 'green' : 'orange'}>{meta.aligned ? '已对齐' : '待对齐'}</Tag> : null}
    </Space>
    <Space wrap style={{ marginTop: 6 }}>
      <PayloadWindows id={id} marker={field(plan)} label="查看本轮计划" />
      {Array.from({ length: subTurns }, (_, sub) => <Space key={sub} wrap>
        <PayloadWindows id={id} marker={field(`team.turn.${number}.sub.${sub}.summary`)} label={`第 ${sub + 1} 次小结`} />
        {participants.map((member) => <PayloadWindows key={member} id={id} marker={field(`team.turn.${number}.sub.${sub}.result.${member}`)} label={`${member} · 第 ${sub + 1} 次结果`} />)}
      </Space>)}
      <InlineFields id={id} value={turn.meta} />
    </Space>
  </div>;
}

function asyncPath(prefix, rowsKey, nextKey) {
  return async (cursor) => {
    if (cursor === null) return { rows: [], next: null };
    const result = await apiGet(`${prefix}${encodeURIComponent(cursor)}`);
    return { rows: result[rowsKey] || [], next: result[nextKey] ?? null };
  };
}

function todoInitializationNotice(detail) {
  const state = detail?.workflow_initialization || (detail?.workflow_initializing ? 'initializing' : '');
  return {
    initializing: ['info', 'TODO 工作流正在初始化', '节点已接受任务，详情准备完成后会自动刷新。'],
    stopping: ['warning', 'TODO 工作流正在停止', '节点已接收停止请求，工作流尚未完成初始化。'],
    stopped: ['info', 'TODO 工作流未启动', '执行已停止，未创建工作流详情。'],
    failed: ['error', 'TODO 工作流初始化失败', detail?.error || '请查看执行错误并按提示处理。'],
  }[state] || null;
}

function TodoDetail({ id, detail }) {
  const initialization = todoInitializationNotice(detail);
  if (!detail?.workflow && initialization) {
    return <div><Typography.Title level={5}>TODO 工作流</Typography.Title>
      <Alert
        type={initialization[0]}
        showIcon
        title={initialization[1]}
        description={initialization[2]}
      />
    </div>;
  }
  const workflow = detail?.workflow?.workflow || detail?.workflow || {};
  const items = detail?.workflow?.items || [];
  const page = detail?.workflow?.items_page || {};
  return <div><Typography.Title level={5}>TODO 工作流</Typography.Title><Descriptions size="small" items={[
    { key: 'status', label: '工作流状态', children: workflow.status || '—' },
    { key: 'progress', label: '当前页', children: `${items.filter((item) => ['completed', 'done'].includes(item.status)).length} / ${items.length} 完成` },
  ]} /><WindowedRows initialRows={items} initialNext={page.next_ordinal}
    path={asyncPath(`/api/executions/${encodeURIComponent(id)}/todo-items?after_ordinal=`, 'items', 'next_ordinal')}
    render={(rows) => <Space orientation="vertical" style={{ width: '100%' }}>{rows.map((item) => <Space key={item.todo_id || item.id} wrap>
      <Tag>{item.todo_id || item.id} · {item.status}</Tag><InlineFields id={id} value={item} />
    </Space>)}</Space>}
  /></div>;
}

function ProjectDetail({ id, detail, onOpen }) {
  const todo = detail?.todo || {};
  const runs = detail?.runs || [];
  const page = detail?.runs_page || {};
  return <div><Typography.Title level={5}>项目任务</Typography.Title><Descriptions size="small" items={[
    { key: 'title', label: '任务', children: todo.title || todo.id || '—' },
    { key: 'status', label: '任务状态', children: todo.status || '—' },
    { key: 'runs', label: '当前页执行', children: `${runs.length} 次` },
  ]} />{typeof todo.plan_md === 'string' ? <Markdown text={todo.plan_md} /> : <Typography.Text type="secondary">尚未生成计划或内容需分段查看</Typography.Text>}
  <WindowedRows initialRows={runs} initialNext={page.next_version}
    path={asyncPath(`/api/executions/${encodeURIComponent(id)}/project-runs?before_version=`, 'runs', 'next_version')}
    render={(rows) => <Space orientation="vertical" style={{ width: '100%' }}>{rows.map((run) => <Space key={run.id || run.version} wrap>
      <Button type="link" onClick={() => onOpen?.(run.id)}>v{run.version} · {run.kind} · {run.status}</Button><InlineFields id={id} value={run} />
    </Space>)}</Space>}
  /></div>;
}

export function WorkloadDetail({ id, detail, kind, onOpen }) {
  if (!detail) return null;
  if (kind === 'team') return <TeamDetail id={id} detail={detail} />;
  if (kind === 'todos') return <TodoDetail id={id} detail={detail} />;
  if (kind === 'project') return detail.run ? null : <ProjectDetail id={id} detail={detail} onOpen={onOpen} />;
  if (kind === 'dag') {
    const definition = detail?.definition?.spec || detail?.definition || {};
    return <Descriptions size="small" items={[
      { key: 'name', label: '工作流', children: definition.name || '—' },
      { key: 'steps', label: '步骤数', children: definition.steps?.length ?? 0 },
    ]} />;
  }
  return null;
}
