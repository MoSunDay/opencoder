// 执行明细内嵌 TODO 运行画布：拉 /api/todo/workflows/:id（{workflow, items}）+
// SSE /events 直通 todo_* 帧折叠进投影，复用工作台的 TodoRunCanvas/Inspector。
// 终帧本地常量（不从 todoRunsPanel 导入，避免 detail.jsx ↔ todoRunsPanel.jsx 循环依赖）。
// 拉取失败或 spec 为空 → 返回 null：分体部署下 node 侧 store 不可见是正常形态，
// 错误静默（不打 onNotice），明细里 workloads.jsx 的 TodoDetail 列表仍在下方回退。
import { useEffect, useMemo, useState } from 'react';
import { Typography } from 'antd';
import { apiGet } from '../../api.js';
import { openStream } from '../../sse.js';
import { foldTodoEvents, itemsToStates, runProgress } from '../../todo/runProjection.js';
import { TodoRunCanvas, TodoRunInspector } from '../../todo/runCanvas.jsx';

const TERMINAL_KINDS = ['workflow_completed', 'workflow_failed'];

export function TodoRunEmbed({ id }) {
  const [detail, setDetail] = useState(null); const [failed, setFailed] = useState(false);
  const [selectedTodoId, setSelectedTodoId] = useState(''); const [liveFrames, setLiveFrames] = useState([]);
  const load = async (silent) => { try { const j = await apiGet(`/api/todo/workflows/${encodeURIComponent(id)}`); setDetail(j || null); setFailed(false); if (silent) setLiveFrames([]); } catch { setFailed(true); } };
  useEffect(() => { setDetail(null); setFailed(false); setSelectedTodoId(''); setLiveFrames([]); load(false);
    let stopped = false;
    const handle = openStream({ path: `/api/todo/workflows/${encodeURIComponent(id)}/events`, after: 0,
      onFrame: (f) => { if (stopped) return; const kind = (f && f.event) || '';
        if (kind.startsWith('todo_')) setLiveFrames((fs) => fs.concat(f));
        else if (kind === 'workflow_rewound' || kind === 'workflow_suspended' || kind === 'workflow_resumed') load(true);
        if (TERMINAL_KINDS.includes(kind)) { handle.abort(); load(true); } } });
    return () => { stopped = true; handle.abort(); };
  }, [id]);
  const wf = (detail && detail.workflow) || null; const items = (detail && detail.items) || [];
  const spec = wf && wf.spec_json && typeof wf.spec_json === 'object' ? wf.spec_json : null;
  const states = useMemo(() => foldTodoEvents(itemsToStates(items), liveFrames), [items, liveFrames]);
  const progress = spec ? runProgress(spec, states) : null;
  const selectedTodo = spec && Array.isArray(spec.todos) ? spec.todos.find((t) => t && t.id === selectedTodoId) : null;
  if (failed || !spec) return null;
  return <div style={{ marginTop: 16 }}>
    <Typography.Title level={5}>TODO 调度画布</Typography.Title>
    {progress && <Typography.Text type="secondary">{progress.passed}/{progress.total} 已通过{progress.active ? ` · ${progress.active} 执行中` : ''}{progress.failed ? ` · ${progress.failed} 失败` : ''}{progress.pending ? ` · ${progress.pending} 待执行` : ''}</Typography.Text>}
    <TodoRunCanvas spec={spec} states={states} selectedId={selectedTodoId} onSelect={setSelectedTodoId} />
    {selectedTodo ? <TodoRunInspector todo={selectedTodo} state={states.get(selectedTodoId)} /> : null}
  </div>;
}
