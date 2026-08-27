// chat.jsx — Tab 2 「会话交互」: node + dialog selectors, prompt composer,
// signed SSE streaming, interrupt. Two dialog sources:
//   remote  GET /api/nodes/:id/dialogs   (may 404 while that feature lands —
//                                         caught, rendered as an empty list)
//   local   GET /api/sessions?limit=50   (server hides node-task sessions)
// Terminal node-task streams always end in a canonical done/error frame
// (api_nodes_ops.rs post_status closure event), which is what stops the stream.

import { Button, Input, Select, Space, Spin, message } from 'antd';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { apiGet, apiPost } from './api.js';
import { openStream } from './sse.js';
import { dialogLabel, relTime } from './format.js';
import { emptyStream, reduceFrame, turnsFromMessages, withUserTurn } from './reduce.js';
import { TranscriptView } from './render.jsx';
import { LOCAL_NODE, LOCAL_NODE_LABEL, clearPreselect, useStore } from './store.js';

const { TextArea } = Input;

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

  const streamRef = useRef(null);
  const lastTaskRef = useRef(new Map()); // dialogKey -> {task_id, session_id}
  const aliveRef = useRef(true);

  const isLocal = nodeSel === LOCAL_NODE;
  const nodeOptions = useMemo(() => (
    [{ value: LOCAL_NODE, label: LOCAL_NODE_LABEL }]
      .concat((nodes || []).map((n) => ({ value: n.id, label: n.name || n.id })))
  ), [nodes]);

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

  const startStream = useCallback(({ path, sessionId, after, initialTurns, initialUsage }) => {
    if (streamRef.current) {
      streamRef.current.abort();
    }
    setStream({
      ...emptyStream(),
      turns: initialTurns || [],
      usage: initialUsage || null,
      status: 'streaming',
    });
    streamRef.current = openStream({
      path,
      sessionId,
      after: after || 0,
      onFrame: (f) => {
        setConnecting(false);
        setStream((s) => reduceFrame(s, f, Date.now()));
      },
      onStatus: (st) => {
        if (st === 'failed') {
          setConnecting(false);
          setBusy(false);
          if (onNotice) {
            onNotice('SSE 流连接失败（已重试 5 次）');
          }
        }
      },
    });
  }, [onNotice]);

  /// Normalize the transcript from the store once a run reaches `done` —
  /// mirrors the vanilla frontend's done → loadTranscript(). Kept best-effort:
  /// if the fetch fails or returns nothing we keep the streamed turns.
  const reloadAfterDone = useCallback(async (sid, currentTurns) => {
    try {
      const j = await apiGet('/api/sessions/' + encodeURIComponent(sid));
      const msgs = (j && j.messages) || [];
      if (msgs.length && aliveRef.current) {
        setStream((s) => ({ ...s, turns: turnsFromMessages(msgs) }));
      }
    } catch {
      setStream((s) => ({ ...s, turns: s.turns.length ? s.turns : currentTurns }));
    }
  }, []);

  const sendLocal = async (prompt) => {
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
    // Snapshot the persisted head BEFORE posting so this stream carries only
    // this turn's events (api_events.rs get_event_seq doc), then open SSE.
    let after = 0;
    try {
      const q = await apiGet('/api/sessions/' + encodeURIComponent(sid) + '/seq');
      after = (q && q.seq) || 0;
    } catch {
      after = 0;
    }
    const ack = await apiPost('/api/sessions/' + encodeURIComponent(sid) + '/prompt', { prompt });
    if (ack && ack.ok === false) {
      throw new Error(ack.error || 'prompt 被拒绝');
    }
    startStream({ path: '/api/sessions/' + encodeURIComponent(sid) + '/events', sessionId: sid, after });
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
    if (dialogSel && sessionId !== dialogSel) {
      // Dispatch created a fresh synthetic session — surface it in the list.
      setDialogs((d) => [{
        session_id: sessionId, title: prompt.slice(0, 40),
        first_created_at: Date.now(), last_created_at: Date.now(), task_count: 1,
      }].concat(d));
      setDialogSel(sessionId);
    }
    setStream((s) => withUserTurn(s, prompt)); // remote has no queue_consumed echo
    startStream({ path: '/api/nodes/tasks/' + encodeURIComponent(taskId) + '/events', sessionId, after: 0 });
  };

  const send = async () => {
    const prompt = input.trim();
    if (!prompt || busy) {
      return;
    }
    setInput('');
    setBusy(true);
    setConnecting(true);
    try {
      if (isLocal) {
        await sendLocal(prompt);
      } else {
        await sendRemote(prompt);
      }
    } catch (e) {
      setConnecting(false);
      setBusy(false);
      setStream((s) => ({ ...s, status: 'error', error: (e && e.message) || '发送失败' }));
    }
  };

  // done → normalize transcript (a terminal frame always stops the stream).
  useEffect(() => {
    if (stream.status !== 'done' || !dialogSel) {
      return;
    }
    setBusy(false);
    setConnecting(false);
    reloadAfterDone(dialogSel, stream.turns);
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
      const turns = turnsFromMessages((j && j.messages) || []);
      if (aliveRef.current) {
        setStream({ ...emptyStream(), turns });
      }
    } catch {
      // Snapshot unavailable → fall back to replaying the last task's events
      // from after=0 (the stream endpoint supports full replay).
      const lt = lastTaskRef.current.get(dialogKey(nodeSel, sid)) || lastTaskRef.current.get(dialogKey(nodeSel, null));
      if (lt && aliveRef.current) {
        setConnecting(true);
        setBusy(true);
        startStream({ path: '/api/nodes/tasks/' + encodeURIComponent(lt.task_id) + '/events', sessionId: lt.session_id, after: 0 });
      }
    }
  };

  const dialogOptions = dialogs.map((d) => ({
    value: d.session_id || d.id,
    label: dialogLabel(d) + ((d.last_created_at || d.last_updated_at) ? '  ·  ' + relTime(d.last_created_at || d.last_updated_at) : ''),
  }));

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      <Space wrap style={{ marginBottom: 12 }}>
        <Select
          style={{ minWidth: 220 }}
          value={nodeSel}
          onChange={setNodeSel}
          options={nodeOptions}
          showSearch
          optionFilterProp="label"
        />
        <Select
          style={{ minWidth: 260 }}
          placeholder="选择对话"
          value={dialogSel}
          onChange={openDialog}
          options={dialogOptions}
          loading={dialogsLoading}
          allowClear
          showSearch
          optionFilterProp="label"
        />
        <Button
          onClick={() => { resetTranscript(); setDialogSel(null); }}
        >
          新建对话
        </Button>
      </Space>

      <div style={{ flex: 1, minHeight: 0, overflow: 'auto', border: '1px solid #f0f0f0', borderRadius: 8, padding: '8px 16px' }}>
        <Spin spinning={connecting} tip="等待首个事件…">
          <TranscriptView
            turns={stream.turns}
            usage={stream.usage}
            status={stream.status}
            error={stream.error}
            emptyText={dialogSel ? '该对话暂无消息，输入提示词开始' : '选择或新建对话，输入提示词开始'}
          />
        </Spin>
      </div>

      <div style={{ marginTop: 12 }}>
        <TextArea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.ctrlKey && e.key === 'Enter') {
              e.preventDefault();
              send();
            }
          }}
          placeholder="输入提示词，Ctrl+Enter 发送"
          autoSize={{ minRows: 2, maxRows: 6 }}
        />
        <Space style={{ marginTop: 8 }}>
          <Button type="primary" onClick={send} disabled={!input.trim() || busy} loading={connecting}>
            发送
          </Button>
          <Button danger onClick={interrupt} disabled={!busy}>
            中断
          </Button>
        </Space>
      </div>
    </div>
  );
}
