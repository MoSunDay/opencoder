// todoDrawer.jsx — 项目 TODO 详情抽屉: 基本信息 / 草稿(可保存) / 执行计划
// (Markdown + 重新生成Plan) / 执行记录 Timeline（v{version}·kind·status、时间
// 区间、plan/output 快照 Collapse、session 复制与跳转、运行中可取消）/
// 删除。Runs come from GET /api/project/todos/:id/runs (newest version first)
// and poll every 3s while any run is still running.

import { Button, Collapse, Drawer, Input, Popconfirm, Space, Spin, Timeline, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPatch, apiPost } from '../api.js';
import { setState } from '../store.js';
import { absTime } from '../format.js';
import { RunStatusTag, TodoStatusTag, runKindLabel } from './labels.jsx';
import { Markdown } from './markdown.jsx';
import { flattenTodos } from './todosTab.jsx';

const { TextArea } = Input;
const { Text, Paragraph } = Typography;

const RUN_POLL_MS = 3000;
const todoPath = (id) => '/api/project/todos/' + encodeURIComponent(id);

function RunItem({ run, onNotice, refreshRuns }) {
  const [cancelling, setCancelling] = useState(false);
  const cancel = async () => {
    setCancelling(true);
    try {
      const j = await apiPost('/api/project/runs/' + encodeURIComponent(run.id) + '/cancel');
      onNotice(j && j.cancelled ? '运行已请求取消' : '该运行已结束，无需取消');
      refreshRuns();
    } catch (e) {
      onNotice('取消失败: ' + (e && e.message));
    } finally {
      setCancelling(false);
    }
  };
  const openSession = () => {
    setState({ page: 'chat' });
    onNotice(`会话 ${run.session_id} 可在「会话交互」中打开`);
  };
  const snaps = [];
  if (run.plan_md) {
    snaps.push({ key: 'plan', label: '计划快照', children: <Markdown text={run.plan_md} /> });
  }
  if (run.output_md) {
    snaps.push({ key: 'out', label: '执行输出', children: <Markdown text={run.output_md} /> });
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
        <Text type="secondary">会话</Text>
        {run.session_id ? (
          <>
            <Text copyable={{ text: run.session_id }} style={{ fontFamily: 'monospace' }}>
              {String(run.session_id).slice(0, 12)}…
            </Text>
            <Button type="link" size="small" onClick={openSession}>查看会话</Button>
          </>
        ) : <Text type="secondary">—</Text>}
        {run.status === 'running' ? (
          <Button type="link" size="small" danger loading={cancelling} onClick={cancel}>取消</Button>
        ) : null}
      </Space>
    </div>
  );
}

export function TodoDrawer({ todoId, overview, refresh, onClose, onNotice }) {
  const [runs, setRuns] = useState([]);
  const [draft, setDraft] = useState(null); // local edit buffer, null = unchanged
  const [acting, setActing] = useState(false);
  const timer = useRef(null);
  const alive = useRef(true);

  const todo = flattenTodos(overview).find((t) => t.id === todoId) || null;
  const anyRunning = runs.some((r) => r.status === 'running');

  // Stable loadRuns: the onNotice prop identity must not re-arm the runs
  // effect (same inline-arrow guard as ProjectPanel.load).
  const noticeRef = useRef(onNotice);
  useEffect(() => {
    noticeRef.current = onNotice;
  }, [onNotice]);

  const loadRuns = useCallback(async (silent) => {
    if (!todoId) {
      return;
    }
    try {
      const j = await apiGet(todoPath(todoId) + '/runs');
      if (alive.current) {
        setRuns((j && j.runs) || []);
      }
    } catch (e) {
      if (!silent && alive.current) {
        const notify = noticeRef.current;
        if (notify) {
          notify('获取执行记录失败: ' + (e && e.message));
        }
      }
    }
  }, [todoId]);

  // Reset local state when the drawer switches to another todo.
  useEffect(() => {
    setRuns([]);
    setDraft(null);
  }, [todoId]);

  // Load on open; re-poll every 3s ONLY while a run is in flight (the
  // interval re-arms when anyRunning flips, mirroring the nodes tab).
  useEffect(() => {
    alive.current = true;
    loadRuns(false);
    if (!anyRunning) {
      return undefined;
    }
    timer.current = setInterval(() => loadRuns(true), RUN_POLL_MS);
    return () => {
      alive.current = false;
      clearInterval(timer.current);
    };
  }, [todoId, anyRunning, loadRuns]);

  // Draft buffer follows the record until the user edits it.
  useEffect(() => {
    setDraft(null);
  }, [todoId, todo && todo.updated_at]);

  const saveDraft = async () => {
    if (!todo || draft === null) {
      return;
    }
    setActing(true);
    try {
      await apiPatch(todoPath(todo.id), { draft });
      onNotice('草稿已保存');
      setDraft(null);
      await refresh();
    } catch (e) {
      onNotice('保存草稿失败: ' + (e && e.message));
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
      await apiPost(todoPath(todo.id) + '/plan');
      onNotice('已开始重新生成 Plan');
      await Promise.all([loadRuns(true), refresh()]);
    } catch (e) {
      onNotice('生成 Plan 失败: ' + (e && e.message));
    } finally {
      setActing(false);
    }
  };

  const remove = async () => {
    try {
      await apiDel(todoPath(todo.id));
      onNotice('TODO 已删除');
      onClose();
      refresh();
    } catch (e) {
      onNotice('删除 TODO 失败: ' + (e && e.message));
    }
  };

  const milestoneLabel = todo && todo.milestone_title ? todo.milestone_title : '未分组';
  const draftValue = draft === null ? (todo ? todo.draft : '') : draft;
  const dirty = draft !== null && todo && draft !== (todo.draft || '');

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
    content: <RunItem run={r} onNotice={onNotice} refreshRuns={() => loadRuns(true)} />,
  }));

  return (
    <Drawer
      open={Boolean(todoId)}
      size={680}
      title={todo ? `TODO · ${todo.title}` : 'TODO 详情'}
      onClose={onClose}
      destroyOnHidden
    >
      {todo ? (
        <Space orientation="vertical" style={{ width: '100%' }} size={20}>
          <div>
            <Paragraph style={{ marginBottom: 4 }}><Text type="secondary">基本信息</Text></Paragraph>
            <Space size={8} wrap>
              <Text strong>{todo.title}</Text>
              <TodoStatusTag status={todo.status} />
              <Text type="secondary">里程碑：{milestoneLabel}</Text>
              <Text type="secondary">agent：{todo.agent || 'act'}</Text>
            </Space>
          </div>
          <div>
            <Paragraph style={{ marginBottom: 4 }}><Text type="secondary">草稿</Text></Paragraph>
            <TextArea
              value={draftValue}
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
            <Markdown text={todo.plan_md} />
            <Button
              size="small"
              style={{ marginTop: 8 }}
              disabled={anyRunning}
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
            {timelineItems.length
              ? <Timeline mode="start" items={timelineItems} />
              : <Text type="secondary">还没有 Plan / 执行记录</Text>}
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
    </Drawer>
  );
}
