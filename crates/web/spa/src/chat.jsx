// Node-owned conversations. Two creation lanes share this console:
//   - Operator 模式（缺省）: operator-kind executions — newId('operator'), body
//       without `kind` (legacy wire shape);
//   - Agent 模式: agent-kind sessions — newId('agent'), body carries
//     `kind: 'agent'` plus the staged `how_append` knowledge addendum.
// Everything else (dialog list, transcript, act/plan, @ menu, model /
// compact / fork / interrupt) is lane-agnostic and reused verbatim. Selection
// is required before creating or sending.
// This panel is the console's chat entry (nav label「Agent」, page key `chat`).
import { Sender } from '@ant-design/x';
import { Alert, Button, Input, Modal, Segmented, Space, Spin, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useLocalStorage } from 'usehooks-ts';
import { apiDel, apiGet, apiPost } from './api.js';
import { canUseNode, newId } from './fleet/model.js';
import { useNodes } from './fleet/useNodes.js';
import { useTranscriptStream } from './chat/useTranscriptStream.js';
import { consumedEchoText, emptyStream, turnsFromMessages, usageFromMessages } from './reduce.js';
import { TranscriptView } from './transcript.jsx';
import { DialogSidebar } from './chatSidebar.jsx';
import { QueuePanel } from './queuePanel.jsx';
import { QuestionModal } from './questionModal.jsx';
import { ModelModal } from './modelModal.jsx';
import { commandsForInput, replaceToken, stripLastToken } from './commandMenu.js';
import { mergeBuiltinPrimaryAgentCards } from './agents/builtins.js';
import { clearPreselect, useStore } from './store.js';
import { err, ok, warn } from './notice.js';
import { MONO_VAR } from './ui/mono.js';

const { Text } = Typography;

/// Agent 模式创建链路的「知识追加 how_append」上限（UTF-8 字节数）。staged 值
/// 只随创建请求（POST /api/sessions）提交；超限一律拦截，绝不随请求外发。
export const HOW_APPEND_MAX = 8192;

export function howAppendBytes(text) {
  return new TextEncoder().encode(typeof text === 'string' ? text : '').length;
}

/// chat 模式持久化键（usehooks-ts useLocalStorage，与 nav 的 oc_nav_page 同
/// 约定）：'operator' | 'agent'，缺省 'operator'。陌生/损坏值在读取处收敛为
/// operator（modeKind），不写回覆盖。
export const CHAT_MODE_STORAGE_KEY = 'oc_chat_mode';

const MODE_OPTIONS = [
  { label: 'Operator 模式', value: 'operator' },
  { label: 'Agent 模式', value: 'agent' },
];

export function ChatPanel({ onNotice }) {
  const { preselectNode } = useStore();
  const { nodes, error: nodesError } = useNodes();
  const [nodeSel, setNodeSel] = useState(null);
  const [dialogs, setDialogs] = useState([]);
  const [dialogSel, setDialogSel] = useState(null);
  const [dialogsLoading, setDialogsLoading] = useState(false);
  const [stream, setStream] = useState(emptyStream);
  const [busy, setBusy] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [input, setInput] = useState('');
  const [queueVersion, setQueueVersion] = useState(0);
  const [skills, setSkills] = useState([]);
  const [agents, setAgents] = useState([]);
  const [sessionAgent, setSessionAgent] = useState('act');
  const [modelOpen, setModelOpen] = useState(false);
  const [apOpen, setApOpen] = useState(false);
  const [annoOpen, setAnnoOpen] = useState(false);
  const [annoText, setAnnoText] = useState('');
  // 创建链路模式（Operator / Agent）与 Agent 模式下的知识追加暂存。mode 经
  // useLocalStorage 持久化（CHAT_MODE_STORAGE_KEY）；howAppend 只在 Agent
  // 模式、且只在「创建」时随请求提交——选中已有会话的发送不受影响。
  const [mode, setMode] = useLocalStorage(CHAT_MODE_STORAGE_KEY, 'operator');
  const [howOpen, setHowOpen] = useState(false);
  const [howText, setHowText] = useState('');
  const [howStaged, setHowStaged] = useState('');

  const streamRef = useRef(null);
  const createAttempt = useRef(null);
  const sendingRef = useRef(false);
  const aliveRef = useRef(true);

  const hasNode = !!nodeSel;
  // 创建链路执行类型：Agent 模式用 agent（节点 kinds 过滤 + newId('agent') +
  // body.kind），其余一律收敛为 Operator 现状（含陌生持久化值）。
  const modeKind = mode === 'agent' ? 'agent' : 'operator';
  const nodeReady = canUseNode(nodes, nodeSel, modeKind);
  const howStagedBytes = howAppendBytes(howStaged);

  const selectionRef = useRef({ node: nodeSel, dialog: dialogSel });
  selectionRef.current = { node: nodeSel, dialog: dialogSel };

  // Tab 1's 打开对话 lands here with a preselected node.
  useEffect(() => {
    if (preselectNode) {
      setNodeSel(preselectNode);
      clearPreselect();
    }
  }, [preselectNode]);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
      if (streamRef.current) {
        streamRef.current.abort();
      }
    };
  }, []);

  // Refresh command completions whenever the selected execution node changes.
  useEffect(() => {
    let alive = true;
    setSkills([]);
    if (!nodeSel) return undefined;
    apiGet('/api/skills?node_id=' + encodeURIComponent(nodeSel)).then((j) => {
      if (alive) {
        setSkills((j && j.skills) || []);
      }
    }).catch((e) => { if (alive) onNotice?.(err('读取节点技能失败: ' + e.message)); });
    return () => {
      alive = false;
    };
  }, [nodeSel, onNotice]);

  // Agent picker catalog source: GET /api/agents reference cards with the
  // server-computed one-line description (prompt-pool soul.md first line).
  // Cards are server-global (not node-scoped like skills), so one fetch per
  // mount; builtin primary roles merge in through agents/builtins.js.
  useEffect(() => {
    let alive = true;
    apiGet('/api/agents').then((j) => {
      if (alive) {
        setAgents((j && j.agents) || []);
      }
    }).catch((e) => { if (alive) onNotice?.(err('读取 agent 列表失败: ' + e.message)); });
    return () => {
      alive = false;
    };
  }, [onNotice]);

  const loadDialogs = useCallback(async (nodeId) => {
    setDialogs([]);
    if (!nodeId) { setDialogsLoading(false); return; }
    setDialogsLoading(true);
    try {
      const j = await apiGet('/api/nodes/' + encodeURIComponent(nodeId) + '/dialogs');
      if (aliveRef.current && selectionRef.current.node === nodeId) setDialogs(j?.dialogs || []);
    } catch (e) {
      if (aliveRef.current && selectionRef.current.node === nodeId) {
        onNotice?.(err('获取会话失败: ' + e.message));
      }
    } finally {
      if (aliveRef.current && selectionRef.current.node === nodeId) setDialogsLoading(false);
    }
  }, [onNotice]);

  const resetTranscript = useCallback(() => {
    if (streamRef.current) {
      streamRef.current.abort();
      streamRef.current = null;
    }
    setStream(emptyStream());
    setBusy(false);
    setConnecting(false);
  }, []);

  useEffect(() => {
    resetTranscript();
    setDialogSel(null);
    setModelOpen(false); setApOpen(false); setAnnoOpen(false); setSessionAgent('act');
    createAttempt.current = null;
    loadDialogs(nodeSel);
  }, [nodeSel, resetTranscript, loadDialogs]);

  // 模式切换只影响下一次创建（进行中的流与已选会话不动）；重置未完成的创建
  // 尝试，避免跨模式复用旧 id（attempt key 含模式，双保险）。
  useEffect(() => {
    createAttempt.current = null;
    setHowOpen(false);
  }, [modeKind]);

  const { reloadAfterDone, openSessionStream } = useTranscriptStream({ streamRef, aliveRef, setStream, setBusy, setConnecting, setQueueVersion, onNotice, selectionRef });

  const sendSession = async (prompt, delivery) => {
    let sid = dialogSel;
    if (!sid) {
      // Creation lanes: Agent 模式 → newId('agent') + body.kind + staged
      // how_append；Operator 模式维持现状（newId('operator')，body 不带 kind）。
      // attempt key 含模式：跨模式重试不复用旧 id。
      createAttempt.current ||= { key: nodeSel + '|' + modeKind, id: newId(modeKind) };
      if (modeKind === 'agent' && howStagedBytes > HOW_APPEND_MAX) {
        // 超限的知识追加绝不随创建提交：拦截发送并提示修复（send 会恢复草稿）。
        throw new Error(`知识追加超过 ${HOW_APPEND_MAX} 字节上限（当前 ${howStagedBytes} 字节），请修改或清空后再发送`);
      }
      // The staged act/plan choice rides creation: POST /api/sessions accepts
      // `agent`, so the mode picked before any prompt exists is honored
      // (server stamps meta.agent and initializes the harness with it).
      const body = { id: createAttempt.current.id, node_id: nodeSel, agent: sessionAgent };
      if (modeKind === 'agent') {
        body.kind = 'agent';
        if (howStaged) body.how_append = howStaged;
      }
      const j = await apiPost('/api/sessions', body);
      if (!j?.id) throw new Error('服务未返回会话 ID，请重试确认');
      createAttempt.current = null;
      sid = j.id;
      selectionRef.current = { node: nodeSel, dialog: sid };
      setDialogSel(sid);
      setDialogs((d) => [{
        session_id: sid, title: prompt.slice(0, 40),
        first_created_at: Date.now(), last_created_at: Date.now(), task_count: null,
      }].concat(d));
    }
    // Snapshot the persisted head BEFORE the POST: if /seq is fetched after
    // the prompt is admitted, events emitted in between get seq ≤ head and
    // are never replayed — this turn's first frames would be lost forever.
    const q = await apiGet('/api/sessions/' + encodeURIComponent(sid) + '/seq');
    const after = q?.seq || 0;
    const ack = await apiPost('/api/sessions/' + encodeURIComponent(sid) + '/prompt',
      { prompt, delivery: delivery === 'queue' ? 'queue' : 'steer' });
    if (ack && ack.ok === false) {
      throw new Error(ack.error || 'prompt 被拒绝');
    }
    // Optimistic echo, injected THROUGH the stream reset (TUI push_user
    // parity): a fresh run carries no steer/queue echo frame, so this echo is
    // the run's only user anchor and must render immediately — no waiting on
    // server frames. consumedEchoText applies the echo contract: compound
    // control commands echo only their tail; a bare control command echoes
    // nothing → no bubble at all. `optimistic` marks the turn as a LOCAL
    // prediction: a later steer/queue_consumed frame echoing the SAME text
    // folds into it instead of pushing a duplicate (reduce.js dedup).
    const echo = consumedEchoText(prompt);
    await openSessionStream(sid, after, [...stream.turns, ...(echo
      ? [{ kind: 'text', role: 'user', text: echo, optimistic: true }]
      : [])]);
  };

  const send = async (rawPrompt, delivery) => {
    const prompt = (typeof rawPrompt === 'string' && rawPrompt.trim()) || input.trim();
    if (!prompt || sendingRef.current) {
      return;
    }
    if (!nodeReady) {
      onNotice?.(warn(nodeSel ? '所选节点当前不可执行，请选择可用节点' : '请先选择执行节点'));
      return;
    }
    if (busy && dialogSel) {
      // A drain is already streaming: admit the prompt on the live session —
      // the runner takes it at the next boundary and the OPEN stream carries
      // the queue/steer echo. Never restart the stream here (startStream
      // resets the transcript, wiping the run in progress).
      setInput('');
      try {
        await apiPost('/api/sessions/' + encodeURIComponent(dialogSel) + '/prompt',
          { prompt, delivery: delivery === 'queue' ? 'queue' : 'steer' });
      } catch (e) {
        if (onNotice) {
          onNotice(err('发送失败: ' + ((e && e.message) || '')));
        }
        setInput(prompt);
      }
      return;
    }
    if (busy) {
      // Session creation/admission is still pending; preserve the draft.
      // Return WITHOUT clearing the composer — clearing here used to swallow
      // the typed input with no notice and no recovery path.
      return;
    }
    sendingRef.current = true;
    setInput('');
    setBusy(true);
    setConnecting(true);
    try {
      await sendSession(prompt, delivery);
    } catch (e) {
      setConnecting(false);
      setBusy(false);
      setInput(prompt);
      setStream((s) => ({ ...s, status: 'error', error: (e && e.message) || '发送失败' }));
    } finally { sendingRef.current = false; }
  };

  // done/error → a terminal frame always stops the stream: release the
  // composer. error previously left busy latched forever (Sender loading,
  // questionModal polling a dead stream); P1-4 makes server error frames
  // reliable, so the terminal path must reset too. The transcript reload
  // stays done-only: a failed run must not clobber what is already shown.
  // (Lag-marked errors never reach here — reduce.js keeps them non-terminal.)
  useEffect(() => {
    if (stream.status !== 'done' && stream.status !== 'error') {
      return;
    }
    setBusy(false);
    setConnecting(false);
    if (stream.status === 'done' && dialogSel) {
      reloadAfterDone(dialogSel, stream.turns);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stream.status]);

  const interrupt = async () => {
    if (!dialogSel) return;
    try { await apiPost('/api/sessions/' + encodeURIComponent(dialogSel) + '/interrupt'); }
    catch (e) { onNotice?.(err('中断失败: ' + e.message)); }
  };

  const openDialog = async (sid) => {
    if (busy) return;
    selectionRef.current = { node: nodeSel, dialog: sid };
    const owner = nodeSel;
    setDialogSel(sid);
    resetTranscript();
    if (!sid) {
      return;
    }
    try {
      const j = await apiGet('/api/sessions/' + encodeURIComponent(sid));
      const msgs = (j && j.messages) || [];
      const agent = j && j.meta && j.meta.agent;
      if (aliveRef.current && selectionRef.current.node === owner && selectionRef.current.dialog === sid) {
        setSessionAgent(agent === 'plan' ? 'plan' : 'act');
        setStream({ ...emptyStream(), turns: turnsFromMessages(msgs), usage: usageFromMessages(msgs) });
      }
    } catch (e) {
      if (aliveRef.current && selectionRef.current.node === owner && selectionRef.current.dialog === sid) {
        setStream((s) => ({ ...s, status: 'error', error: '读取会话失败: ' + e.message }));
      }
    }
  };

  const notice = (msg) => {
    if (onNotice) {
      onNotice(msg);
    }
  };

  /// Session commands are relayed to the selected conversation owner. agent/
  /// agentpick (@name)/compact open a drain-facing POST; the picker kinds
  /// just open their modal; 'text' kinds ride the normal prompt path.
  const execCommand = async (entry) => {
    const kind = entry && entry.kind;
    const sid = dialogSel;
    if (kind === 'agent' || kind === 'agentpick') {
      const next = String((entry && entry.value) || '');
      if (!next) {
        return;
      }
      // Busy control heads are TEXT prompts for the runner: applied at the
      // next turn boundary while a drain runs (control_cmd.rs parity). A
      // custom agent rides the generic `/agent <name>` head; /act //plan are
      // heads of their own. Posted directly — send() would wipe the composer
      // draft (setInput('')).
      const headText = kind === 'agentpick' ? '/agent ' + next : entry.cmd;
      if (busy && sid) {
        try {
          await apiPost('/api/sessions/' + encodeURIComponent(sid) + '/prompt',
            { prompt: headText, delivery: 'steer' });
        } catch (e) {
          notice(err('切换 agent 失败: ' + ((e && e.message) || '')));
        }
        return;
      }
      if (!sid) {
        // No session yet: stage the agent locally — it rides creation
        // (POST /api/sessions `agent`) with the next prompt.
        setSessionAgent(next);
        return;
      }
      try {
        await apiPost('/api/sessions/' + encodeURIComponent(sid) + '/agent', { value: next });
        setSessionAgent(next);
      } catch (e) {
        notice(err('切换 agent 失败: ' + ((e && e.message) || '')));
      }
      return;
    }
    if (kind === 'compact') {
      if (!sid) {
        notice(warn('先发送一条提示词新建对话'));
        return;
      }
      try {
        // Same pre-POST snapshot as sendSession: Compaction/TranscriptReset
        // frames emitted between the POST ack and a late /seq fetch would be
        // skipped forever.
        const q = await apiGet('/api/sessions/' + encodeURIComponent(sid) + '/seq');
        const after = q?.seq || 0;
        await apiPost('/api/sessions/' + encodeURIComponent(sid) + '/compact');
        setBusy(true);
        setConnecting(true);
        await openSessionStream(sid, after); // compaction deltas arrive on the stream
      } catch (e) {
        setConnecting(false);
        setBusy(false);
        notice(err('压缩失败: ' + ((e && e.message) || '')));
      }
      return;
    }
    if (kind === 'model') {
      setModelOpen(true);
      return;
    }
    if (kind === 'ap') {
      setApOpen(true);
      return;
    }
    if (kind === 'annotation') {
      setAnnoOpen(true);
      return;
    }
    if (kind === 'fork') {
      if (!sid) {
        notice(warn('先发送一条提示词新建对话'));
        return;
      }
      try {
        const j = await apiPost('/api/sessions/' + encodeURIComponent(sid) + '/fork');
        if (j && j.id) {
          setDialogs((d) => [{
            session_id: j.id, title: 'fork · ' + sid.slice(0, 12),
            first_created_at: Date.now(), last_created_at: Date.now(), task_count: null,
          }].concat(d.filter((x) => x.session_id !== j.id)));
          await openDialog(j.id);
        }
      } catch (e) {
        notice(err('fork 失败: ' + ((e && e.message) || '')));
      }
      return;
    }
    // 'text' (and anything unknown): the runner consumes it as a prompt.
    send(entry.cmd, 'steer');
  };

  const switchAgent = (value) => {
    execCommand({ kind: 'agent', cmd: '/' + value, value });
  };

  /// Sidebar hover 删除 → confirm → DELETE /api/sessions/:id. The server
  /// cascades messages/events/inputs and cancels any running drain, so no
  /// client-side interrupt is needed before the call.
  const deleteDialog = (sid) => {
    if (!sid) {
      return;
    }
    Modal.confirm({
      title: '删除会话',
      content: '删除后该会话的消息、事件与队列输入将一并清除。',
      okText: '删除',
      okButtonProps: { danger: true },
      cancelText: '取消',
      onOk: async () => {
        try {
          await apiDel('/api/sessions/' + encodeURIComponent(sid));
          if (sid === dialogSel) {
            resetTranscript();
            setDialogSel(null);
          }
          setDialogs((d) => d.filter((x) => x.session_id !== sid));
          notice(ok('会话已删除'));
        } catch (e) {
          notice(err('删除会话失败: ' + ((e && e.message) || '')));
        }
      },
    });
  };

  /// Bulk clear-all: DELETE /api/nodes/:id/dialogs removes every TERMINAL
  /// (done | error | cancelled) dialog of the selected node in one call; the
  /// server SKIPS sessions whose node task is still pending/running/cancelling
  /// so running work survives the sweep. Secondary confirm mirrors the
  /// per-dialog deleteDialog gate.
  const deleteAllDialogs = () => {
    if (!nodeSel || !dialogs.length) {
      return;
    }
    Modal.confirm({
      title: '删除全部会话',
      content: '将删除当前节点的所有已完成会话（消息、事件与队列输入一并清除）；正在运行中的会话会保留。',
      okText: '全部删除',
      okButtonProps: { danger: true },
      cancelText: '取消',
      onOk: async () => {
        try {
          const j = await apiDel('/api/nodes/' + encodeURIComponent(nodeSel) + '/dialogs');
          const skipped = (j && j.skipped) || [];
          if (dialogSel && !skipped.includes(dialogSel)) {
            resetTranscript();
            setDialogSel(null);
          }
          loadDialogs(nodeSel);
          const removed = (j && j.removed) || 0;
          notice(ok(skipped.length
            ? `已删除 ${removed} 个会话，${skipped.length} 个运行中的会话已保留`
            : `已删除 ${removed} 个会话`));
        } catch (e) {
          notice(err('批量删除会话失败: ' + ((e && e.message) || '')));
        }
      },
    });
  };

  const setAutopilot = async (mode) => {
    setApOpen(false);
    if (!dialogSel) {
      notice(warn('先发送一条提示词新建对话'));
      return;
    }
    try {
      await apiPost('/api/sessions/' + encodeURIComponent(dialogSel) + '/autopilot', { mode });
      notice(ok(mode ? 'autopilot → ' + mode : 'autopilot 已清除'));
    } catch (e) {
      notice(err('autopilot 设置失败: ' + ((e && e.message) || '')));
    }
  };

  const setAnnotation = async (text) => {
    setAnnoOpen(false);
    if (!dialogSel) {
      notice(warn('先发送一条提示词新建对话'));
      return;
    }
    try {
      // AnnotationBody { text: Option<String> } — blank means CLEAR.
      await apiPost('/api/sessions/' + encodeURIComponent(dialogSel) + '/annotation', { text });
      notice(ok(text ? '批注已保存' : '批注已清除'));
    } catch (e) {
      notice(err('批注保存失败: ' + ((e && e.message) || '')));
    }
  };

  /// Menu click: skills and the `/agent` command entry complete the token in
  /// place; agent picks (`@name`) and everything else wipe the token from the
  /// composer and execute immediately.
  const pickCommand = (entry) => {
    if (!entry) {
      return;
    }
    if (entry.kind === 'skill' || entry.kind === 'agentcmd') {
      setInput((t) => replaceToken(t, entry));
      return;
    }
    setInput((t) => stripLastToken(t));
    execCommand(entry);
  };

  // Command completion uses the selected node’s catalog.
  // `@` agent entries: builtin primary roles first, then the resolvable
  // primary cards from GET /api/agents — the switch endpoint
  // (POST /api/sessions/:id/agent) rejects non-primary names, so the menu
  // only ever offers switchable agents.
  const agentCatalog = mergeBuiltinPrimaryAgentCards((agents || []).filter((a) => a && a.primary));
  const menuEntries = hasNode ? commandsForInput(input, skills, agentCatalog) : [];

  return (
    <div style={{ display: 'flex', flexDirection: 'row', height: '100%', minHeight: 0, gap: 16 }}>
      <DialogSidebar
        nodes={nodes}
        nodeSel={nodeSel}
        nodeKind={modeKind}
        onNodeChange={setNodeSel}
        disabled={busy}
        dialogs={dialogs}
        activeKey={dialogSel}
        onActiveChange={openDialog}
        onDelete={deleteDialog}
        onDeleteAll={deleteAllDialogs}
        loading={dialogsLoading}
      />

      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
        {/* 页头操作区：模式 Segmented（Operator / Agent，持久化于 oc_chat_mode）
            常驻；Agent 模式追加「知识追加」暂存入口；选中节点后再出现 act/plan
            与 模型（会话级控制，语义不变）。 */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
          <Segmented
            aria-label="会话模式"
            size="small"
            value={modeKind}
            options={MODE_OPTIONS}
            onChange={setMode}
          />
          {modeKind === 'agent' ? (
            <Button size="small" onClick={() => { setHowText(howStaged); setHowOpen(true); }}>
              知识追加{howStaged ? ' · 已暂存' : ''}
            </Button>
          ) : null}
          {hasNode ? (
            <>
              <Segmented
                aria-label="agent 切换"
                size="small"
                value={sessionAgent}
                options={[{ label: 'act', value: 'act' }, { label: 'plan', value: 'plan' }]}
                onChange={switchAgent}
              />
              <Button size="small" disabled={!dialogSel} onClick={() => setModelOpen(true)}>模型</Button>
            </>
          ) : null}
        </div>

        {!nodeReady && <Alert type="info" showIcon style={{ marginBottom: 12 }} title={nodesError || (nodeSel ? '所选节点当前不可执行，请选择可用节点' : '请先选择执行节点')} />}
        <div style={{ flex: 1, minHeight: 0, overflow: 'auto', border: '1px solid var(--oc-border)', borderRadius: 10, padding: '12px 16px' }}>
          <Spin spinning={connecting} description="等待首个事件…">
            <TranscriptView
              turns={stream.turns}
              usage={stream.usage}
              status={stream.status}
              error={stream.error}
              emptyText={dialogSel ? '该对话暂无消息，输入提示词开始' : '选中节点后输入提示词，即新建对话'}
            />
          </Spin>
        </div>

        <QueuePanel sessionId={hasNode ? dialogSel : null} refreshSignal={queueVersion} />

        <div style={{ marginTop: 12, position: 'relative' }}>
          {menuEntries.length > 0 ? (
            <div
              style={{
                position: 'absolute', bottom: '100%', left: 0, right: 0, marginBottom: 4,
                background: 'var(--oc-panel-bg)', border: '1px solid var(--oc-border)', borderRadius: 8,
                boxShadow: '0 4px 16px rgba(0,0,0,0.08)', zIndex: 20,
                maxHeight: 264, overflow: 'auto',
              }}
            >
              {menuEntries.map((entry) => (
                <div
                  key={entry.cmd}
                  data-cmd={entry.cmd}
                  style={{ padding: '6px 12px', cursor: 'pointer', display: 'flex', gap: 8, alignItems: 'baseline' }}
                  onClick={() => pickCommand(entry)}
                >
                  <Text strong style={{ fontFamily: MONO_VAR }}>{entry.cmd}</Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>{entry.desc}</Text>
                </div>
              ))}
            </div>
          ) : null}
          <div style={{ display: 'flex', gap: 8, alignItems: 'flex-start' }}>
            <div style={{ flex: 1, minWidth: 0 }}>
              <Sender
                value={input}
                onChange={setInput}
                onSubmit={send}
                onCancel={interrupt}
                loading={busy}
                disabled={!nodeReady}
                placeholder="输入提示词，Enter 发送，Shift+Enter 换行"
              />
            </div>
          </div>
        </div>
      </div>

      <ModelModal open={modelOpen} sessionId={dialogSel} nodeId={nodeSel} onClose={() => setModelOpen(false)} onNotice={notice} />

      <Modal
        title="autopilot 模式"
        open={apOpen}
        footer={null}
        onCancel={() => setApOpen(false)}
      >
        <Space wrap>
          <Button onClick={() => setAutopilot('off')}>off</Button>
          <Button onClick={() => setAutopilot('ap')}>ap</Button>
          <Button onClick={() => setAutopilot('review')}>review</Button>
          <Button onClick={() => setAutopilot(null)}>清除</Button>
        </Space>
      </Modal>

      <Modal
        title="设置批注"
        open={annoOpen}
        footer={null}
        onCancel={() => setAnnoOpen(false)}
      >
        <Input
          value={annoText}
          placeholder="批注内容（留空保存即清除）"
          onChange={(e) => setAnnoText(e.target.value)}
          onPressEnter={() => setAnnotation(annoText.trim())}
        />
        <Space style={{ marginTop: 12 }}>
          <Button type="primary" onClick={() => setAnnotation(annoText.trim())}>保存</Button>
          <Button onClick={() => setAnnotation('')}>清除</Button>
        </Space>
      </Modal>

      {/* Agent 模式「知识追加 how_append」暂存：保存即 staged，随下一次创建
          （POST /api/sessions）提交；超限（HOW_APPEND_MAX 字节）禁止保存并在
          创建前再拦一道。Operator 模式不渲染入口。 */}
      <Modal
        title="知识追加（how_append）"
        open={howOpen}
        footer={null}
        onCancel={() => setHowOpen(false)}
      >
        <Input.TextArea
          aria-label="知识追加内容"
          value={howText}
          rows={5}
          placeholder="追加给本次 agent 会话的知识/上下文（随创建提交，留空即不携带）"
          onChange={(e) => setHowText(e.target.value)}
        />
        <Text type={howAppendBytes(howText) > HOW_APPEND_MAX ? 'danger' : 'secondary'} style={{ fontSize: 12, display: 'block', marginTop: 4 }}>
          {howAppendBytes(howText)} / {HOW_APPEND_MAX} 字节
          {howAppendBytes(howText) > HOW_APPEND_MAX ? ' · 超过上限，无法保存' : ''}
        </Text>
        <Space style={{ marginTop: 12 }}>
          <Button
            type="primary"
            disabled={howAppendBytes(howText) > HOW_APPEND_MAX}
            onClick={() => { setHowStaged(howText); setHowOpen(false); notice(ok('知识追加已暂存，将随下一次创建提交')); }}
          >
            保存
          </Button>
          <Button onClick={() => { setHowText(''); setHowStaged(''); setHowOpen(false); notice(ok('知识追加已清空')); }}>清空</Button>
          <Button onClick={() => setHowOpen(false)}>取消</Button>
        </Space>
      </Modal>

      <QuestionModal sessionId={hasNode ? dialogSel : null} active={busy} />
    </div>
  );
}
