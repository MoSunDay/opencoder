// Node-owned conversations: selection is required before creating or sending.
import { Sender } from '@ant-design/x';
import { Alert, Button, Input, Modal, Segmented, Space, Spin, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from './api.js';
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
import { clearPreselect, useStore } from './store.js';
import { err, ok, warn } from './notice.js';

const { Text } = Typography;

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
  const [sessionAgent, setSessionAgent] = useState('act');
  const [modelOpen, setModelOpen] = useState(false);
  const [apOpen, setApOpen] = useState(false);
  const [annoOpen, setAnnoOpen] = useState(false);
  const [annoText, setAnnoText] = useState('');

  const streamRef = useRef(null);
  const createAttempt = useRef(null);
  const sendingRef = useRef(false);
  const aliveRef = useRef(true);

  const hasNode = !!nodeSel;
  const nodeReady = canUseNode(nodes, nodeSel, 'agent');
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

  const { reloadAfterDone, openSessionStream } = useTranscriptStream({ streamRef, aliveRef, setStream, setBusy, setConnecting, setQueueVersion, onNotice, selectionRef });

  const sendSession = async (prompt, delivery) => {
    let sid = dialogSel;
    if (!sid) {
      createAttempt.current ||= { key: nodeSel, id: newId('agent') };
      const j = await apiPost('/api/sessions', { id: createAttempt.current.id, node_id: nodeSel });
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

  /// Session commands are relayed to the selected conversation owner. agent/compact open a drain-facing POST; the picker
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
        notice(err('切换模式失败: ' + ((e && e.message) || '')));
      }
      return;
    }
    if (kind === 'compact') {
      if (!sid) {
        notice(warn('先选择或新建对话'));
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
        notice(warn('先选择或新建对话'));
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

  const setAutopilot = async (mode) => {
    setApOpen(false);
    if (!dialogSel) {
      notice(warn('先选择或新建对话'));
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
      notice(warn('先选择或新建对话'));
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

  // Command completion uses the selected node’s catalog.
  const menuEntries = hasNode ? commandsForInput(input, skills) : [];

  return (
    <div style={{ display: 'flex', flexDirection: 'row', height: '100%', minHeight: 0 }}>
      <DialogSidebar
        nodes={nodes}
        nodeSel={nodeSel}
        onNodeChange={setNodeSel}
        disabled={busy}
        dialogs={dialogs}
        activeKey={dialogSel}
        onActiveChange={openDialog}
        onNew={() => { if (busy || !nodeReady) return; resetTranscript(); setDialogSel(null); setSessionAgent('act'); createAttempt.current = null; }}
        loading={dialogsLoading}
      />

      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
        {hasNode ? (
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

        {!nodeReady && <Alert type="info" showIcon style={{ marginBottom: 8 }} title={nodesError || (nodeSel ? '所选节点当前不可执行，请选择可用节点' : '请先选择执行节点')} />}
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

        <QueuePanel sessionId={hasNode ? dialogSel : null} refreshSignal={queueVersion} />

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
                disabled={!nodeReady}
                placeholder="输入提示词，Enter 发送，Shift+Enter 换行"
              />
            </div>
            <Button
              style={{ height: 40 }}
              disabled={!nodeReady || !dialogSel || !input.trim()}
              onClick={() => send(input.trim(), 'queue')}
            >
              排队
            </Button>
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
