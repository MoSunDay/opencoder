// todoDrawer.jsx — 项目 TODO 详情抽屉: 基本信息 / 草稿(可保存) / 执行计划
// (Markdown + 重新生成Plan) / 执行记录 Timeline（v{version}·kind·status、时间
// 区间、plan/output 快照 Collapse、session 复制与跳转、运行中可取消）/
// 删除。Runs come from GET /api/project/todos/:id/runs (newest version first)
// and poll every 3s while any run is still running.

import { Alert, Button, Collapse, Drawer, Input, Popconfirm, Space, Spin, Timeline, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../api.js';
import { submitAttempt } from './replay/attempt.js';
import { useRuns } from './replay/useRuns.js';
import { RunText } from './replay/run.jsx';
import { absTime } from '../format.js';
import { RunStatusTag, TodoStatusTag, ExecutorTag, runKindLabel } from './labels.jsx';
import { RelationSelect } from './views/relationSelect.jsx';
import { flattenTodos, milestoneOptions } from './model/relations.js';
import { ExecutionDetail } from '../fleet/detail.jsx';
import { err, info, ok, warn } from '../notice.js';

const { TextArea } = Input;
const { Text, Paragraph } = Typography;

const todoPath = (id) => '/api/project/todos/' + encodeURIComponent(id);

function RunItem({ run, executionId, onNotice, refreshRuns, openExecution }) {
  const [cancelling, setCancelling] = useState(false);
  const cancel = async () => {
    setCancelling(true);
    try {
      await apiPost('/api/executions/' + encodeURIComponent(executionId) + '/commands', { action: 'cancel', input: {} });
      onNotice(warn('执行已取消，不能恢复'));
      refreshRuns();
    } catch (e) {
      onNotice(err('取消失败: ' + (e && e.message)));
    } finally {
      setCancelling(false);
    }
  };
  const snaps = [];
  if (run.plan_md) {
    snaps.push({ key: 'plan', label: '计划快照', children: <RunText id={run.id} value={run.plan_md} /> });
  }
  if (run.output_md) {
    snaps.push({ key: 'out', label: '执行输出', children: <RunText id={run.id} value={run.output_md} /> });
  }
  return (
    <div className="proj-run-card">
      <Paragraph style={{ marginBottom: 4 }}>
        <Text type="secondary">
          {run.started_at ? absTime(run.started_at) : '—'} → {run.finished_at ? absTime(run.finished_at) : '进行中'}
        </Text>
      </Paragraph>
      {snaps.length ? <Collapse size="small" items={snaps} /> : <Text type="secondary">无快照输出</Text>}
      <Space size={4} wrap style={{ marginTop: 8 }}>
        <Button size="small" onClick={() => openExecution(run.id)}>查看本次输入与过程</Button>
        <ExecutorTag kind={run.executor_kind} />
        {run.capability_id ? (
          <Text type="secondary" style={{ fontSize: 12 }}>
            brain: {run.capability_id}{run.plan_id ? ` · ${String(run.plan_id).slice(0, 12)}…` : ''}
          </Text>
        ) : null}
        {run.output_ref ? (
          <Text
            type="secondary"
            title={run.output_ref}
            style={{ fontFamily: 'monospace', fontSize: 12 }}
          >
            {String(run.output_ref).length > 24 ? String(run.output_ref).slice(0, 24) + '…' : run.output_ref}
          </Text>
        ) : null}
        <Text type="secondary">会话</Text>
        {run.session_id ? (
          <>
            <Text copyable={{ text: run.session_id }} style={{ fontFamily: 'monospace' }}>
              {String(run.session_id).slice(0, 12)}…
            </Text>
            <Button type="link" size="small" onClick={() => openExecution(run.session_id)}>查看会话</Button>
          </>
        ) : <Text type="secondary">—</Text>}
        {run.status === 'running' ? (
          <Button type="link" size="small" danger loading={cancelling} onClick={cancel}>取消</Button>
        ) : null}
      </Space>
    </div>
  );
}

function TodoDrawerSession({ todoId, overview, refresh, onClose, onNotice }) {
  const [agent, setAgent] = useState(null);
  const [draft, setDraft] = useState(null); // local edit buffer, null = unchanged
  const [savedDraft, setSavedDraft] = useState(null);
  const [acting, setActing] = useState(false);
  const [executionOpen, setExecutionOpen] = useState(null);
  const todo = flattenTodos(overview).find((t) => t.id === todoId) || null;
  const running = ['pending', 'running', 'cancelling'].includes(todo?.execution?.status);
  const history = useRuns(todoId, running);
  const { runs } = history;
  const loadRuns = history.refresh;
  const anyRunning = running || runs.some((r) => r.status === 'running');
  const executionClosed = ['cancelled', 'done'].includes(todo?.execution?.status);
  useEffect(() => { setAgent(null); setExecutionOpen(null); }, [todoId]);
  const saveAgent = async () => {
    setActing(true);
    try {
      await apiPatch(todoPath(todoId), { agent: agent.trim() });
      await refresh(); onNotice(ok('执行 Agent 已更新，下次执行使用当前版本'));
    } catch (e) { onNotice(err(e.message)); }
    finally { setActing(false); }
  };

  // Draft buffer follows the record until the user edits it.
  useEffect(() => {
    setDraft(null);
  }, [todoId]);

  const saveDraft = async () => {
    if (!todo || draft === null) {
      return;
    }
    setActing(true);
    try {
      await apiPatch(todoPath(todo.id), { draft });
      onNotice(ok('草稿已保存'));
      setSavedDraft(draft);
      await refresh();
    } catch (e) {
      onNotice(err('保存草稿失败: ' + (e && e.message)));
    } finally {
      setActing(false);
    }
  };

  const genPlan = async () => {
    if (!todo) {
      return;
    }
    setActing(true);
    try {
      await submitAttempt(todo.id, 'plan');
      onNotice(info('已开始重新生成 Plan'));
      await Promise.all([loadRuns(true), refresh()]);
    } catch (e) {
      onNotice(err('生成 Plan 失败: ' + (e && e.message)));
    } finally {
      setActing(false);
    }
  };

  const remove = async () => {
    try {
      await apiDel(todoPath(todo.id));
      onNotice(ok('TODO 已删除'));
      onClose();
      refresh();
    } catch (e) {
      onNotice(err('删除 TODO 失败: ' + (e && e.message)));
    }
  };

  const draftValue = draft === null ? (todo ? todo.draft : '') : draft;
  const dirty = draft !== null && todo && draft !== (savedDraft ?? todo.draft ?? '');

  const timelineItems = runs.map((r) => ({
    key: r.id,
    color: r.status === 'failed' ? 'red' : r.status === 'running' ? 'blue' : r.status === 'done' ? 'green' : 'gray',
    icon: r.status === 'running' ? <Spin size="small" /> : undefined,
    title: (
      <Space size={8}>
        <Text strong>v{r.version}</Text>
        <Text type="secondary">· {runKindLabel(r.kind)} ·</Text>
        <RunStatusTag status={r.status} />
      </Space>
    ),
    content: <RunItem run={r} executionId={`project-${todoId}`} onNotice={onNotice} refreshRuns={loadRuns} openExecution={setExecutionOpen} />,
  }));

  return (
    <Drawer
      open={Boolean(todoId)}
      size={680}
      title={todo ? `TODO · ${todo.title}` : 'TODO 详情'}
      onClose={() => { if (!acting) onClose(); }}
      destroyOnHidden
    >
      {todo ? (
        <Space orientation="vertical" style={{ width: '100%' }} size={20}>
          <div>
            <Paragraph style={{ marginBottom: 4 }}><Text type="secondary">基本信息</Text></Paragraph>
            <Space size={8} wrap>
              <Text strong>{todo.title}</Text>
              <TodoStatusTag status={todo.status} />
              <RelationSelect path={todoPath(todo.id)} field="milestone_id" value={todo.milestone_id}
                options={milestoneOptions(overview)} refresh={refresh} onNotice={onNotice} label="TODO 所属里程碑" disabled={acting} />
              <Text type="secondary">项目：{todo.goal_title || '未关联'}</Text>
              <Text type="secondary">agent：{todo.agent || 'act'}</Text>
            </Space>
            {todo.executor_kind === 'agent' && <Space style={{ marginTop: 8 }}>
              <Input aria-label="执行 Agent" value={agent ?? todo.agent ?? 'act'} disabled={anyRunning || acting} onChange={(e) => setAgent(e.target.value)} />
              <Button disabled={anyRunning || !agent?.trim() || agent === todo.agent} loading={acting} onClick={saveAgent}>保存 Agent</Button>
            </Space>}
            {todo.execution ? <Button size="small" style={{ marginTop: 8 }} onClick={() => setExecutionOpen(`project-${todoId}`)}>查看节点执行详情</Button> : null}
            {executionClosed ? <Alert type="warning" showIcon style={{ marginTop: 8 }} title="该节点执行已终止，不能再次生成或执行计划" /> : null}
          </div>
          <div>
            <Paragraph style={{ marginBottom: 4 }}><Text type="secondary">草稿</Text></Paragraph>
            <TextArea
              value={draftValue}
              disabled={acting}
              autoSize={{ minRows: 3, maxRows: 12 }}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="粗略描述要做什么…"
              aria-label="todo-draft"
            />
            <Button
              size="small"
              type="primary"
              style={{ marginTop: 8 }}
              disabled={!dirty}
              loading={acting && dirty}
              onClick={saveDraft}
            >
              保存草稿
            </Button>
          </div>
          <div>
            <Paragraph style={{ marginBottom: 4 }}><Text type="secondary">执行计划（plan_md）</Text></Paragraph>
            <RunText id={`project-${todoId}`} value={todo.plan_md} />
            <Button
              size="small"
              style={{ marginTop: 8 }}
              disabled={anyRunning || executionClosed}
              loading={acting && !dirty}
              onClick={genPlan}
            >
              重新生成Plan
            </Button>
          </div>
          <div>
            <Paragraph style={{ marginBottom: 8 }}>
              <Text type="secondary">执行记录（新版本在前）</Text>
            </Paragraph>
            {history.error && <Alert type="error" showIcon title={history.error} />}
            {history.updated && <Text type="secondary">最近成功读取：{absTime(history.updated)}</Text>}
            {timelineItems.length
              ? <Timeline mode="start" items={timelineItems} />
              : <Text type="secondary">还没有 Plan / 执行记录</Text>}
            {history.more && <Button loading={history.busy} onClick={history.next}>加载更早记录</Button>}
          </div>
          <div>
            <Popconfirm
              title="删除该 TODO？"
              description="将一并删除其执行记录。"
              okText="删除"
              okButtonProps={{ danger: true }}
              cancelText="取消"
              onConfirm={remove}
            >
              <Button danger>删除 TODO</Button>
            </Popconfirm>
          </div>
        </Space>
      ) : (
        <Text type="secondary">未找到该 TODO（可能已被删除）</Text>
      )}
      {executionOpen && <ExecutionDetail id={executionOpen} summary={executionOpen === `project-${todoId}` ? todo?.execution : null} onClose={() => setExecutionOpen(null)} onNotice={onNotice} />}
    </Drawer>
  );
}

// A new record is a new editing session; late responses only touch the old instance.
export function TodoDrawer(props) {
  return <TodoDrawerSession key={props.todoId || "closed"} {...props} />;
}
