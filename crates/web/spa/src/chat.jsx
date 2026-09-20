// Node-owned conversations. Two creation lanes share this console:
//   - Operator 模式（缺省）: operator-kind executions — newId('operator'), body
//       without `kind` (legacy wire shape);
//   - Agent 模式: agent-kind sessions — newId('agent'), body carries
//     `kind: 'agent'` plus the selected concrete Agent. The first prompt is
//     persisted into that Agent's how by the worker.
// Everything else (dialog list, transcript, act/plan, @ menu, model /
// compact / fork / interrupt) is lane-agnostic and reused verbatim. Selection
// is required before creating or sending.
// This panel is the console's chat entry (nav label「Agent」, page key `chat`).
import { Sender } from '@ant-design/x';
import { Alert, Button, Input, Modal, Segmented, Select, Space, Spin, Tag, Tooltip, Typography } from 'antd';
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
import { BUILTIN_AGENT_HEADS, mergeBuiltinPrimaryAgentCards } from './agents/builtins.js';
import { RUN_MODE_HINT, runModeBadge } from './agents/runMode.js';
import { clearPreselect, useStore } from './store.js';
import { err, ok, warn } from './notice.js';
import { MONO_VAR } from './ui/mono.js';

const { Text } = Typography;

/// Agent how_append 的 UTF-8 字节上限，保留为公共常量供调用方校验。
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
  // 创建链路模式（Operator / Agent）。mode 经 useLocalStorage 持久化；
  // Agent 首条 prompt 由 worker 自动写入选中的 Agent how。
  const [mode, setMode] = useLocalStorage(CHAT_MODE_STORAGE_KEY, 'operator');

  const streamRef = useRef(null);
  const createAttempt = useRef(null);
  const sendingRef = useRef(false);
  const aliveRef = useRef(true);
  const dialogsRequestRef = useRef(0);

  const hasNode = !!nodeSel;
  // 创建链路执行类型：Agent 模式用 agent（节点 kinds 过滤 + newId('agent') +
  // body.kind），其余一律收敛为 Operator 现状（含陌生持久化值）。
  const modeKind = mode === 'agent' ? 'agent' : 'operator';
  const laneRef = useRef({ node: nodeSel, kind: modeKind });
  if (laneRef.current.node !== nodeSel || laneRef.current.kind !== modeKind) {
    laneRef.current = { node: nodeSel, kind: modeKind };
  }
  const lane = laneRef.current;
  const isCurrentLane = () => aliveRef.current && laneRef.current === lane;
  const nodeReady = canUseNode(nodes, nodeSel, modeKind);
  // Agent 模式「执行 Agent」的可选集：只认 Agent 配置（GET /api/agents）里
  // primary 的注册卡。内置 act/plan/command 是 operator 宿主循环的角色，
  // 不进入 Agent 执行泳道的下拉；agentCatalog（内置在前 + 注册卡）只服务
  // Operator 模式的切换面（`@` 菜单与选中兜底）。
  const registeredAgents = (agents || []).filter((a) => a && a.primary);
  const agentCatalog = mergeBuiltinPrimaryAgentCards(registeredAgents);
  const selectedAgent = registeredAgents.find((agent) => agent.name === sessionAgent);

  const selectionRef = useRef({ node: nodeSel, kind: modeKind, dialog: dialogSel });
  selectionRef.current = { node: nodeSel, kind: modeKind, dialog: dialogSel };

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
  // mount. The Agent-mode picker consumes this list as-is; only Operator-mode
  // switch surfaces merge the builtin trio in (agents/builtins.js).
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

  const loadDialogs = useCallback(async (nodeId, kind = modeKind) => {
    if (selectionRef.current.node !== nodeId || selectionRef.current.kind !== kind) return;
    const request = ++dialogsRequestRef.current;
    const current = () => aliveRef.current && dialogsRequestRef.current === request
      && selectionRef.current.node === nodeId && selectionRef.current.kind === kind;
    setDialogs([]);
    if (!nodeId) { setDialogsLoading(false); return; }
    setDialogsLoading(true);
    try {
      const j = await apiGet('/api/nodes/' + encodeURIComponent(nodeId) + '/dialogs?kind=' + encodeURIComponent(kind));
      if (current()) setDialogs(j?.dialogs || []);
    } catch (e) {
      if (current()) {
        onNotice?.(err('获取会话失败: ' + e.message));
      }
    } finally {
      if (current()) setDialogsLoading(false);
    }
  }, [modeKind, onNotice]);

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
    setModelOpen(false); setApOpen(false); setAnnoOpen(false);
    setSessionAgent(modeKind === 'agent' ? (registeredAgents[0]?.name || '') : 'act');
    createAttempt.current = null;
    loadDialogs(nodeSel, modeKind);
  }, [nodeSel, modeKind, resetTranscript, loadDialogs]);

  // Keep a concrete selection even while the registry request is loading or
  // when a persisted/remote value no longer resolves to a selectable Agent.
  // The selectable set is mode-scoped: Agent 模式只有注册卡（选不中就收敛到
  // 第一张注册卡；配置为空收敛为 ''，交给发送门禁拦截）；Operator 模式沿用
  // 内置在前 + 注册卡的目录。
  useEffect(() => {
    const catalog = modeKind === 'agent' ? registeredAgents : agentCatalog;
    if (catalog.some((agent) => agent.name === sessionAgent)) return;
    if (catalog[0]) setSessionAgent(catalog[0].name);
    else if (modeKind === 'agent' && sessionAgent) setSessionAgent('');
  }, [agents, sessionAgent, modeKind]);

  const { reloadAfterDone, openSessionStream } = useTranscriptStream({ streamRef, aliveRef, setStream, setBusy, setConnecting, setQueueVersion, onNotice, selectionRef });

  const sendSession = async (prompt, delivery) => {
    let sid = dialogSel;
    if (!sid) {
      // Creation lanes: Agent 模式 → newId('agent') + body.kind + concrete
      // agent；Operator 模式维持现状（newId('operator')，body 不带 kind）。
      // attempt key 含模式：跨模式重试不复用旧 id。
      createAttempt.current ||= { key: nodeSel + '|' + modeKind, id: newId(modeKind) };
      // The staged act/plan choice rides creation: POST /api/sessions accepts
      // `agent`, so the mode picked before any prompt exists is honored
      // (server stamps meta.agent and initializes the harness with it). Agent
      // mode also carries its first prompt in this request. The node can then
      // initialize the session and start the drain atomically; the old
      // create-then-seq-then-prompt chain could race the node's async launch
      // and fail the first Agent submission with a transient 404.
      const body = { id: createAttempt.current.id, node_id: nodeSel, agent: sessionAgent };
      if (modeKind === 'agent') {
        body.kind = 'agent';
        body.prompt = prompt;
      }
      const j = await apiPost('/api/sessions', body);
      if (!j?.id) throw new Error('服务未返回会话 ID，请重试确认');
      sid = j.id;
      if (!isCurrentLane()) return;
      createAttempt.current = null;
      selectionRef.current = { node: nodeSel, kind: modeKind, dialog: sid };
      setDialogSel(sid);
      setDialogs((d) => [{
        session_id: sid, title: prompt.slice(0, 40),
        first_created_at: Date.now(), last_created_at: Date.now(), task_count: null,
      }].concat(d));
    }
    if (!isCurrentLane()) return;
    // A fresh Agent-mode session carries no separate prompt POST: its first
    // prompt rode the creation request, so the cursor stays at 0 and the
    // stream replays the complete new session without a readiness
    // round-trip to the node before its local session row exists.
    let after = 0;
    if (dialogSel || modeKind !== 'agent') {
      // Snapshot the persisted head BEFORE the POST: if /seq is fetched after
      // the prompt is admitted, events emitted in between get seq ≤ head and
      // are never replayed — this turn's first frames would be lost forever.
      const q = await apiGet('/api/sessions/' + encodeURIComponent(sid) + '/seq');
      after = q?.seq || 0;
      const ack = await apiPost('/api/sessions/' + encodeURIComponent(sid) + '/prompt',
        { prompt, delivery: delivery === 'queue' ? 'queue' : 'steer' });
      if (ack && ack.ok === false) {
        throw new Error(ack.error || 'prompt 被拒绝');
      }
    }
    // Optimistic echo, injected THROUGH the stream reset (TUI push_user
    // parity): a fresh run carries no steer/queue echo frame, so this echo is
    // the run's only user anchor and must render immediately — no waiting on
    // server frames. consumedEchoText applies the echo contract: compound
    // control commands echo only their tail; a bare control command echoes
    // nothing → no bubble at all. `optimistic` marks the turn as a LOCAL
    // prediction: a later steer/queue_consumed frame echoing the SAME text
    // folds into it instead of pushing a duplicate (reduce.js dedup).
    if (!isCurrentLane()) return;
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
    // Agent 模式的执行目标必须落在一张配置的注册卡上：内置角色不是该泳道
    // 的可选目标，配置为空时给出去配置的指引，而不是静默回落到内置 act。
    if (modeKind === 'agent' && !registeredAgents.some((agent) => agent.name === sessionAgent)) {
      onNotice?.(warn(registeredAgents.length ? '请先选择执行 Agent' : 'Agent 配置中还没有可用的 Agent，请先在「Agent 配置」页创建'));
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
      if (!isCurrentLane()) return;
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
    selectionRef.current = { node: nodeSel, kind: modeKind, dialog: sid };
    const owner = nodeSel;
    const current = () => aliveRef.current && selectionRef.current.node === owner
      && selectionRef.current.kind === modeKind && selectionRef.current.dialog === sid;
    setDialogSel(sid);
    resetTranscript();
    if (!sid) {
      return;
    }
    try {
      const j = await apiGet('/api/sessions/' + encodeURIComponent(sid));
      const msgs = (j && j.messages) || [];
      const agent = j && j.meta && j.meta.agent;
      if (current()) {
        const agentLane = modeKind === 'agent' ? registeredAgents : agentCatalog;
        setSessionAgent(agentLane.some((item) => item.name === agent) ? agent : agentLane[0]?.name || '');
        setStream({ ...emptyStream(), turns: turnsFromMessages(msgs), usage: usageFromMessages(msgs) });
      }
    } catch (e) {
      if (current()) {
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
    const current = () => aliveRef.current && selectionRef.current.node === nodeSel
      && selectionRef.current.kind === modeKind && selectionRef.current.dialog === sid;
    if (kind === 'agent' || kind === 'agentpick') {
      const next = String((entry && entry.value) || '');
      if (!next) {
        return;
      }
      // Busy control heads are TEXT prompts for the runner: applied at the
      // next turn boundary while a drain runs (control_cmd.rs parity). Only
      // act/plan own their heads (`/act`, `/plan`); every other target —
      // configured cards and the builtin command role — rides the generic
      // `/agent <name>` head. Posted directly — send() would wipe the
      // composer draft (setInput('')).
      const headText = BUILTIN_AGENT_HEADS.includes(next) ? '/' + next : '/agent ' + next;
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
        if (current()) setSessionAgent(next);
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
        if (!current()) return;
        setBusy(true);
        setConnecting(true);
        await openSessionStream(sid, after); // compaction deltas arrive on the stream
      } catch (e) {
        if (!current()) return;
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
        if (j && j.id && current()) {
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
          if (selectionRef.current.kind !== modeKind || selectionRef.current.node !== nodeSel) return;
          if (sid === selectionRef.current.dialog) {
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
      content: `将删除当前节点 ${modeKind === 'agent' ? 'Agent' : 'Operator'} 模式的所有已完成会话（消息、事件与队列输入一并清除）；正在运行中的会话会保留。`,
      okText: '全部删除',
      okButtonProps: { danger: true },
      cancelText: '取消',
      onOk: async () => {
        try {
          const j = await apiDel('/api/nodes/' + encodeURIComponent(nodeSel) + '/dialogs?kind=' + encodeURIComponent(modeKind));
          const skipped = (j && j.skipped) || [];
          if (selectionRef.current.node !== nodeSel || selectionRef.current.kind !== modeKind) return;
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

  // Command completion uses the selected node’s catalog. `@` agent entries
  // follow the mode's selectable set — Agent 模式只列 Agent 配置的注册卡
  // （与「执行 Agent」下拉同源）；Operator 模式保持内置 primary 角色在前 +
  // 注册卡（act/plan 切换与自定义 Agent 提及）。The switch endpoint
  // (POST /api/sessions/:id/agent) rejects non-primary names, so the menu
  // only ever offers switchable agents.
  const menuEntries = hasNode
    ? commandsForInput(input, skills, modeKind === 'agent' ? registeredAgents : agentCatalog)
    : [];

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
        {/* 页头操作区：模式与执行目标分开。Agent 模式必须明确选择一个
            concrete Agent，候选只来自 Agent 配置（GET /api/agents）的
            primary 注册卡——内置 act/plan/command 是 operator 宿主循环的
            角色，不进入该下拉；配置为空时发送被拦截并提示先去配置。
            首条需求会自动进入该 Agent 的 how，并沿用同一 transcript/Say
            渲染，便于在网页中定位具体能力。 */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
          <Segmented
            aria-label="会话模式"
            size="small"
            value={modeKind}
            options={MODE_OPTIONS}
            onChange={setMode}
          />
          {modeKind === 'agent' ? (
            <Select
              aria-label="执行 Agent"
              size="small"
              value={sessionAgent || undefined}
              placeholder={registeredAgents.length ? '选择执行 Agent' : '暂无可用 Agent'}
              options={registeredAgents.map((agent) => ({
                value: agent.name,
                label: agent.name,
                title: agent.description || agent.name,
              }))}
              optionRender={({ data }) => (
                <div>
                  <div>{data.label}</div>
                  {data.title && <Text type="secondary" style={{ fontSize: 12 }}>{data.title}</Text>}
                </div>
              )}
              onChange={switchAgent}
              style={{ minWidth: 150 }}
            />
          ) : null}
          {hasNode ? (
            <>
              {modeKind !== 'agent' ? <Segmented
                  aria-label="agent 切换"
                  size="small"
                  value={sessionAgent}
                  options={[{ label: 'act', value: 'act' }, { label: 'plan', value: 'plan' }]}
                  onChange={switchAgent}
                /> : null}
              {modeKind === 'agent' && selectedAgent ? (
                <Tooltip title={RUN_MODE_HINT}>
                  <Tag aria-label="selected-agent-run-mode" style={{ marginInlineEnd: 0 }}>
                    {runModeBadge(selectedAgent.run_mode)}
                  </Tag>
                </Tooltip>
              ) : null}
              {modeKind === 'agent' && selectedAgent?.description ? (
                <Text type="secondary" ellipsis={{ tooltip: selectedAgent.description }} style={{ maxWidth: 360 }}>
                  {selectedAgent.description}
                </Text>
              ) : null}
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

      <QuestionModal sessionId={hasNode ? dialogSel : null} active={busy} />
    </div>
  );
}
