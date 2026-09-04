// chat.jsx — Tab 2 「会话交互」: node + dialog selectors, prompt composer,
// signed SSE streaming, interrupt. Two dialog sources:
//   remote  GET /api/nodes/:id/dialogs   (may 404 while that feature lands —
//                                         caught, rendered as an empty list)
//   local   GET /api/sessions?limit=50   (server hides node-task sessions)
// Terminal node-task streams always end in a canonical done/error frame
// (api_nodes_ops.rs post_status closure event), which is what stops the stream.
//
// T4 composer note: the input area is now @ant-design/x Sender, which ships
// with submitType='enter' — Enter sends, Shift+Enter inserts a newline and
// Ctrl+Enter does nothing. This intentionally replaces the old
// TextArea + Ctrl+Enter binding; the data flow is unchanged (onSubmit → the
// same send(), loading → stop button wired to the same interrupt()).
//
// T5 layout note: dialog selection moved from a header antd Select into an
// @ant-design/x Conversations sidebar (chatSidebar.jsx) — the classic X chat
// two-column shell. Node switcher rides along on top of the sidebar. All
// handlers (loadDialogs / openDialog / resetTranscript / send / interrupt /
// preselect effect) are unchanged; only their mount points moved. The
// sidebar's activeKey IS dialogSel, and its creation button calls the same
// reset pair the old 新建对话 button did.
//
// T6 wiring note: composer + transcript gain the TUI's remaining surfaces —
//   * slash/$skill command menu (commandMenu.js) with LOCAL execution paths
//     (agent/compact/model/ap/annotation/fork); while a drain runs, /act //
//     /plan are POSTed as TEXT (control_cmd.rs applies them at the boundary)
//   * 排队 button → prompt delivery "queue" (steer stays the Enter default);
//     while a drain streams, send() admits on the live session instead of
//     restarting the stream (a restart resets the transcript view)
//   * QueuePanel (pending inputs) + QuestionModal (question tool poll)
//   * subagent fold blocks render through transcript.jsx → subagentBlock.jsx

import { Sender } from '@ant-design/x';
import { Button, Input, Modal, Segmented, Space, Spin, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from './api.js';
import { openStream } from './sse.js';
import { consumedEchoText, emptyStream, ensurePendingEcho, reduceFrame, resyncState, turnsFromMessages, usageFromMessages, withUserTurn } from './reduce.js';
import { TranscriptView } from './transcript.jsx';
import { DialogSidebar } from './chatSidebar.jsx';
import { QueuePanel } from './queuePanel.jsx';
import { QuestionModal } from './questionModal.jsx';
import { ModelModal } from './modelModal.jsx';
import { commandsForInput, replaceToken, stripLastToken } from './commandMenu.js';
import { LOCAL_NODE, clearPreselect, useStore } from './store.js';

const { Text } = Typography;

function dialogKey(nodeId, sessionId) {
  return nodeId + '|' + (sessionId || '');
}

export function ChatPanel({ onNotice }) {
  const { nodes, preselectNode } = useStore();
  const [nodeSel, setNodeSel] = useState(LOCAL_NODE);
  const [dialogs, setDialogs] = useState([]);
  const [dialogSel, setDialogSel] = useState(null);
  const [dialogsLoading, setDialogsLoading] = useState(false);
  const [stream, setStream] = useState(emptyStream);
  const [busy, setBusy] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [input, setInput] = useState('');
  const [queueVersion, setQueueVersion] = useState(0);
  const [skills, setSkills] = useState([]);
  const [sessionAgent, setSessionAgent] = useState('act');
  const [modelOpen, setModelOpen] = useState(false);
  const [apOpen, setApOpen] = useState(false);
  const [annoOpen, setAnnoOpen] = useState(false);
  const [annoText, setAnnoText] = useState('');

  const streamRef = useRef(null);
  const lastTaskRef = useRef(new Map()); // dialogKey -> {task_id, session_id}
  const aliveRef = useRef(true);

  const isLocal = nodeSel === LOCAL_NODE;

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

  // $-skill completions for the command menu — one fetch per mount, best
  // effort (an empty list only shrinks the menu, never breaks the composer).
  useEffect(() => {
    let alive = true;
    apiGet('/api/skills').then((j) => {
      if (alive) {
        setSkills((j && j.skills) || []);
      }
    }).catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const loadDialogs = useCallback(async (nodeId) => {
    setDialogsLoading(true);
    try {
      if (nodeId === LOCAL_NODE) {
        const j = await apiGet('/api/sessions?limit=50');
        const list = ((j && j.sessions) || []).map((s) => ({
          session_id: s.id,
          title: s.title,
          first_created_at: s.created_at,
          last_created_at: s.updated_at,
          task_count: null,
        }));
        if (aliveRef.current) {
          setDialogs(list);
        }
      } else {
        const j = await apiGet('/api/nodes/' + encodeURIComponent(nodeId) + '/dialogs');
        if (aliveRef.current) {
          setDialogs((j && j.dialogs) || []);
        }
      }
    } catch {
      if (aliveRef.current) {
        setDialogs([]); // dialogs endpoint may not exist yet — never crash
      }
    } finally {
      if (aliveRef.current) {
        setDialogsLoading(false);
      }
    }
  }, []);

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
    loadDialogs(nodeSel);
  }, [nodeSel, resetTranscript, loadDialogs]);

  /// Normalize the transcript from the store once a run reaches `done` —
  /// mirrors the vanilla frontend's done → loadTranscript(). Kept best-effort:
  /// if the fetch fails or returns nothing we keep the streamed turns.
  const reloadAfterDone = useCallback(async (sid, currentTurns) => {
    try {
      const j = await apiGet('/api/sessions/' + encodeURIComponent(sid));
      const msgs = (j && j.messages) || [];
      if (msgs.length && aliveRef.current) {
        setStream((s) => ({
          ...s,
          // TUI /act_clear_context <tail> parity (rebuild_after_reset): the
          // reset fires inside the admitted turn, so the store snapshot has
          // NOT recorded the echo yet — re-push the user boundary if the
          // rebuilt turns lack it. The functional update reads the LIVE
          // pendingEcho; on the done path it is already null and the snapshot
          // itself carries the echo → no-op, behavior unchanged.
          turns: ensurePendingEcho(turnsFromMessages(msgs), s.pendingEcho),
          usage: usageFromMessages(msgs),
        }));
      }
    } catch {
      setStream((s) => ({ ...s, turns: s.turns.length ? s.turns : currentTurns }));
    }
  }, []);

  const startStream = useCallback(({ path, sessionId, after, initialTurns, initialUsage }) => {
    if (streamRef.current) {
      streamRef.current.abort();
    }
    // startStream RESETS the whole stream state, so a caller's optimistic
    // user echo must ride IN via initialTurns (pushing it before this call is
    // a guaranteed wipe). A fresh run never sees a steer/queue echo frame
    // first, so the optimistic turn is the run's ONLY user anchor — mirror
    // the TUI push_user + pending_turn_echo pair: seed pendingEcho from the
    // last initial turn when it is a user text turn (bare control commands
    // echo nothing → empty initialTurns → null), so a transcript_reset
    // rebuild (reloadAfterDone → ensurePendingEcho) re-pushes the boundary.
    const initialList = Array.isArray(initialTurns) ? initialTurns : [];
    const lastInitial = initialList[initialList.length - 1];
    const initialPendingEcho = lastInitial && lastInitial.kind === 'text' && lastInitial.role === 'user'
      ? lastInitial.text
      : null;
    setStream({
      ...emptyStream(),
      turns: initialTurns || [],
      usage: initialUsage || null,
      pendingEcho: initialPendingEcho,
      status: 'streaming',
    });
    streamRef.current = openStream({
      path,
      sessionId,
      after: after || 0,
      onFrame: (f) => {
        setConnecting(false);
        setStream((s) => reduceFrame(s, f, Date.now()));
        // A consumed input frees a QueuePanel row — pull-only refresh.
        if (f && (f.event === 'queue_consumed' || f.event === 'steer_consumed')) {
          setQueueVersion((v) => v + 1);
        }
        // The wire payload is {} (runner/event.rs): only the store snapshot
        // knows the collapsed transcript — refetch immediately.
        if (f && f.event === 'transcript_reset' && sessionId) {
          reloadAfterDone(sessionId, []);
        }
      },
      onStatus: (st, info) => {
        if (st === 'failed') {
          setConnecting(false);
          setBusy(false);
          if (onNotice) {
            onNotice('SSE 流连接失败（已重试 5 次）');
          }
        }
      },
      // Round-2 #5 resync: every reconnect (lag re-sync or retry) rebuilds
      // the fold state from the store snapshot at the /seq watermark instead
      // of folding the replay tail into the dirty live state — live frames
      // carry no seq, so replaying after=lastSeq would re-fold every frame
      // consumed since the last id'd one (doubled text, duplicated tool
      // rows, re-pushed echo turns). The snapshot's `draining` flag also
      // closes the finished-while-disconnected gap: a run that ended during
      // the outage has its terminal frame at seq ≤ head (never replayed), so
      // a non-draining rebuild lands status 'done' and releases busy instead
      // of latching 'streaming' forever. Returns null on failure → sse.js
      // falls back to the capped legacy cursor (today's behavior).
      onResync: async () => {
        if (!sessionId) {
          return null;
        }
        const sid = sessionId;
        try {
          const q = await apiGet('/api/sessions/' + encodeURIComponent(sid) + '/seq');
          const head = q && typeof q.seq === 'number' ? q.seq : 0;
          const j = await apiGet('/api/sessions/' + encodeURIComponent(sid));
          if (!aliveRef.current) {
            return null;
          }
          setStream((s) => resyncState({
            messages: (j && j.messages) || [],
            draining: !!(j && j.draining),
            headSeq: head,
            pendingEcho: s.pendingEcho,
          }));
          return head;
        } catch {
          return null;
        }
      },
    });
  }, [onNotice, reloadAfterDone]);

  /// seq head → signed /events stream (sendLocal + 压缩 share the open path).
  /// `after` is the pre-POST seq head owned by the caller; when omitted we
  /// fall back to fetching /seq here (best-effort, no ordering guarantee —
  /// callers that need only-this-turn's events must snapshot BEFORE posting).
  /// `initialTurns` threads the caller's optimistic echo turns into
  /// startStream's reset state (see startStream's comment).
  const openLocalStream = useCallback(async (sid, after, initialTurns) => {
    let head = after;
    if (head === undefined) {
      try {
        const q = await apiGet('/api/sessions/' + encodeURIComponent(sid) + '/seq');
        head = (q && q.seq) || 0;
      } catch {
        head = 0;
      }
    }
    startStream({
      path: '/api/sessions/' + encodeURIComponent(sid) + '/events',
      sessionId: sid,
      after: head,
      initialTurns,
    });
  }, [startStream]);

  const sendLocal = async (prompt, delivery) => {
    let sid = dialogSel;
    if (!sid) {
      const j = await apiPost('/api/sessions', {});
      sid = j.id;
      setDialogSel(sid);
      setDialogs((d) => [{
        session_id: sid, title: prompt.slice(0, 40),
        first_created_at: Date.now(), last_created_at: Date.now(), task_count: null,
      }].concat(d));
    }
    // Snapshot the persisted head BEFORE the POST: if /seq is fetched after
    // the prompt is admitted, events emitted in between get seq ≤ head and
    // are never replayed — this turn's first frames would be lost forever.
    let after = 0;
    try {
      const q = await apiGet('/api/sessions/' + encodeURIComponent(sid) + '/seq');
      after = (q && q.seq) || 0;
    } catch {
      after = 0;
    }
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
    await openLocalStream(sid, after, echo
      ? [{ kind: 'text', role: 'user', text: echo, optimistic: true }]
      : []);
  };

  const sendRemote = async (prompt) => {
    const body = { prompt };
    // Best-effort resume: current dispatch schema has no session_id field, a
    // future one may. If the server rejects it, retry with a bare prompt.
    if (dialogSel) {
      body.session_id = dialogSel;
    }
    let j;
    try {
      j = await apiPost('/api/nodes/' + encodeURIComponent(nodeSel) + '/tasks', body);
    } catch (e) {
      if (body.session_id && [400, 404, 409, 422, 500].includes(e && e.status)) {
        j = await apiPost('/api/nodes/' + encodeURIComponent(nodeSel) + '/tasks', { prompt });
      } else {
        throw e;
      }
    }
    const taskId = j.task_id;
    const sessionId = j.session_id;
    lastTaskRef.current.set(dialogKey(nodeSel, dialogSel || sessionId), { task_id: taskId, session_id: sessionId });
    if (sessionId !== dialogSel) {
      // Dispatch created a fresh synthetic session — surface it in the list.
      // First dispatch has no dialogSel yet: backfill it too, so the
      // terminal-frame effect sees a selection and the done → store reload
      // actually runs for the session we just streamed.
      setDialogs((d) => [{
        session_id: sessionId, title: prompt.slice(0, 40),
        first_created_at: Date.now(), last_created_at: Date.now(), task_count: 1,
      }].concat(d));
      setDialogSel(sessionId);
    }
    // Remote dispatch has no queue_consumed echo (synthetic task session), so
    // the optimistic user turn applies the echo contract itself: bare control
    // commands render nothing, compounds render only the tail. `optimistic`
    // marks the local prediction (reduce.js dedups a same-text echo frame).
    setStream((s) => withUserTurn(s, consumedEchoText(prompt), true));
    startStream({ path: '/api/nodes/tasks/' + encodeURIComponent(taskId) + '/events', sessionId, after: 0 });
  };

  const send = async (rawPrompt, delivery) => {
    const prompt = (typeof rawPrompt === 'string' && rawPrompt.trim()) || input.trim();
    if (!prompt) {
      return;
    }
    if (busy && isLocal && dialogSel) {
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
          onNotice('发送失败: ' + ((e && e.message) || ''));
        }
        setInput(prompt);
      }
      return;
    }
    if (busy) {
      // Remote busy: nothing can be admitted while the node runs the task.
      // Return WITHOUT clearing the composer — clearing here used to swallow
      // the typed input with no notice and no recovery path.
      return;
    }
    setInput('');
    setBusy(true);
    setConnecting(true);
    try {
      if (isLocal) {
        await sendLocal(prompt, delivery);
      } else {
        await sendRemote(prompt);
      }
    } catch (e) {
      setConnecting(false);
      setBusy(false);
      setStream((s) => ({ ...s, status: 'error', error: (e && e.message) || '发送失败' }));
    }
  };

  // done/error → a terminal frame always stops the stream: release the
  // composer. error previously left busy latched forever (Sender loading,
  // questionModal polling a dead stream); P1-4 makes server error frames
  // reliable, so the terminal path must reset too. The transcript reload
  // stays done-only: a failed run must not clobber what is already shown.
  // (Lag-marked errors never reach here — reduce.js keeps them non-terminal.)
  // The busy release is NOT gated on dialogSel: a first remote dispatch has
  // no selection until sendRemote backfills one, and gating the reset on it
  // used to latch the Sender loading until the user clicked some dialog.
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
    try {
      if (isLocal) {
        if (!dialogSel) {
          return;
        }
        await apiPost('/api/sessions/' + encodeURIComponent(dialogSel) + '/interrupt');
      } else {
        const lt = lastTaskRef.current.get(dialogKey(nodeSel, dialogSel));
        if (!lt) {
          return;
        }
        await apiPost('/api/nodes/' + encodeURIComponent(nodeSel) + '/tasks/' + encodeURIComponent(lt.task_id) + '/cancel');
      }
    } catch (e) {
      if (onNotice) {
        onNotice('中断失败: ' + ((e && e.message) || ''));
      }
    }
  };

  const openDialog = async (sid) => {
    setDialogSel(sid);
    resetTranscript();
    if (!sid) {
      return;
    }
    try {
      const j = await apiGet('/api/sessions/' + encodeURIComponent(sid));
      const msgs = (j && j.messages) || [];
      const agent = j && j.meta && j.meta.agent;
      setSessionAgent(agent === 'plan' ? 'plan' : 'act');
      if (aliveRef.current) {
        setStream({ ...emptyStream(), turns: turnsFromMessages(msgs), usage: usageFromMessages(msgs) });
      }
    } catch {
      // Snapshot unavailable → fall back to replaying the last task's events
      // from after=0 (the stream endpoint supports full replay).
      setSessionAgent('act');
      const lt = lastTaskRef.current.get(dialogKey(nodeSel, sid)) || lastTaskRef.current.get(dialogKey(nodeSel, null));
      if (lt && aliveRef.current) {
        setConnecting(true);
        setBusy(true);
        startStream({ path: '/api/nodes/tasks/' + encodeURIComponent(lt.task_id) + '/events', sessionId: lt.session_id, after: 0 });
      }
    }
  };

  const notice = (msg) => {
    if (onNotice) {
      onNotice(msg);
    }
  };

  /// Slash-command execution (LOCAL only — remote dispatch keeps today's
  /// plain-text behavior). agent/compact open a drain-facing POST; the picker
  /// kinds just open their modal; 'text' kinds ride the normal prompt path.
  const execCommand = async (entry) => {
    const kind = entry && entry.kind;
    const sid = dialogSel;
    if (kind === 'agent') {
      if (busy || !sid) {
        // Control heads are TEXT prompts for the runner: applied at the next
        // turn boundary while a drain runs (control_cmd.rs parity).
        send(entry.cmd, 'steer');
        return;
      }
      try {
        await apiPost('/api/sessions/' + encodeURIComponent(sid) + '/agent', { value: entry.value });
        setSessionAgent(entry.value === 'plan' ? 'plan' : 'act');
      } catch (e) {
        notice('切换模式失败: ' + ((e && e.message) || ''));
      }
      return;
    }
    if (kind === 'compact') {
      if (!sid) {
        notice('先选择或新建对话');
        return;
      }
      try {
        // Same pre-POST snapshot as sendLocal: Compaction/TranscriptReset
        // frames emitted between the POST ack and a late /seq fetch would be
        // skipped forever.
        let after = 0;
        try {
          const q = await apiGet('/api/sessions/' + encodeURIComponent(sid) + '/seq');
          after = (q && q.seq) || 0;
        } catch {
          after = 0;
        }
        await apiPost('/api/sessions/' + encodeURIComponent(sid) + '/compact');
        setBusy(true);
        setConnecting(true);
        await openLocalStream(sid, after); // compaction deltas arrive on the stream
      } catch (e) {
        setConnecting(false);
        setBusy(false);
        notice('压缩失败: ' + ((e && e.message) || ''));
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
        notice('先选择或新建对话');
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
        notice('fork 失败: ' + ((e && e.message) || ''));
      }
      return;
    }
    // 'text' (and anything unknown): the runner consumes it as a prompt.
    send(entry.cmd, 'steer');
  };

  const switchAgent = (value) => {
    execCommand({ kind: 'agent', cmd: '/' + value, value });
  };

  const setAutopilot = async (mode) => {
    setApOpen(false);
    if (!dialogSel) {
      notice('先选择或新建对话');
      return;
    }
    try {
      await apiPost('/api/sessions/' + encodeURIComponent(dialogSel) + '/autopilot', { mode });
      notice(mode ? 'autopilot → ' + mode : 'autopilot 已清除');
    } catch (e) {
      notice('autopilot 设置失败: ' + ((e && e.message) || ''));
    }
  };

  const setAnnotation = async (text) => {
    setAnnoOpen(false);
    if (!dialogSel) {
      notice('先选择或新建对话');
      return;
    }
    try {
      // AnnotationBody { text: Option<String> } — blank means CLEAR.
      await apiPost('/api/sessions/' + encodeURIComponent(dialogSel) + '/annotation', { text });
      notice(text ? '批注已保存' : '批注已清除');
    } catch (e) {
      notice('批注保存失败: ' + ((e && e.message) || ''));
    }
  };

  /// Menu click: skills complete the token in place; everything else wipes
  /// the token from the composer and executes immediately.
  const pickCommand = (entry) => {
    if (!entry) {
      return;
    }
    if (entry.kind === 'skill') {
      setInput((t) => replaceToken(t, entry));
      return;
    }
    setInput((t) => stripLastToken(t));
    execCommand(entry);
  };

  // Command menu items follow the LAST `/…`/`$…` token of the composer text
  // (LOCAL only — remote dispatch has no local execution surface).
  const menuEntries = isLocal ? commandsForInput(input, skills) : [];

  return (
    <div style={{ display: 'flex', flexDirection: 'row', height: '100%', minHeight: 0 }}>
      <DialogSidebar
        nodes={nodes}
        nodeSel={nodeSel}
        onNodeChange={setNodeSel}
        dialogs={dialogs}
        activeKey={dialogSel}
        onActiveChange={openDialog}
        onNew={() => { resetTranscript(); setDialogSel(null); }}
        loading={dialogsLoading}
      />

      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
        {isLocal ? (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
            <Segmented
              size="small"
              value={sessionAgent}
              disabled={!dialogSel}
              options={[{ label: 'act', value: 'act' }, { label: 'plan', value: 'plan' }]}
              onChange={switchAgent}
            />
            <Button size="small" disabled={!dialogSel} onClick={() => setModelOpen(true)}>模型</Button>
            <Button size="small" disabled={!dialogSel} onClick={() => setAnnoOpen(true)}>批注</Button>
            <Button size="small" disabled={!dialogSel} onClick={() => execCommand({ kind: 'compact', cmd: '/compact' })}>压缩</Button>
            <Button size="small" disabled={!dialogSel} onClick={() => setApOpen(true)}>autopilot</Button>
          </div>
        ) : null}

        <div style={{ flex: 1, minHeight: 0, overflow: 'auto', border: '1px solid #f0f0f0', borderRadius: 8, padding: '8px 16px' }}>
          <Spin spinning={connecting} description="等待首个事件…">
            <TranscriptView
              turns={stream.turns}
              usage={stream.usage}
              status={stream.status}
              error={stream.error}
              emptyText={dialogSel ? '该对话暂无消息，输入提示词开始' : '选择或新建对话，输入提示词开始'}
            />
          </Spin>
        </div>

        <QueuePanel sessionId={isLocal ? dialogSel : null} refreshSignal={queueVersion} />

        <div style={{ marginTop: 12, position: 'relative' }}>
          {menuEntries.length > 0 ? (
            <div
              style={{
                position: 'absolute', bottom: '100%', left: 0, right: 0, marginBottom: 4,
                background: '#fff', border: '1px solid #f0f0f0', borderRadius: 8,
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
                  <Text strong style={{ fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace' }}>{entry.cmd}</Text>
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
                placeholder="输入提示词，Enter 发送，Shift+Enter 换行"
              />
            </div>
            <Button
              style={{ height: 40 }}
              disabled={!isLocal || !dialogSel || !input.trim()}
              onClick={() => send(input.trim(), 'queue')}
            >
              排队
            </Button>
          </div>
        </div>
      </div>

      <ModelModal open={modelOpen} sessionId={dialogSel} onClose={() => setModelOpen(false)} onNotice={notice} />

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

      <QuestionModal sessionId={isLocal ? dialogSel : null} active={busy} />
    </div>
  );
}
