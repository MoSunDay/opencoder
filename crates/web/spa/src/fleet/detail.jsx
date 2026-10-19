import { BrainExecutionPresentation } from './detail/presentation/brain.jsx';
import { EditButton } from '../ui/permissions.jsx';
import { ConversationInput } from '../chat/conversationInput.jsx';
import { QueuePanel } from '../queuePanel.jsx';
import { QuestionModal } from '../questionModal.jsx';
import { useCanEdit } from '../ui/permissions.jsx';
import { DagRunResult } from '../dag/run/result.jsx';
import { DagRunContext } from '../dag/run/context.jsx';
import { Alert, Button, Collapse, Descriptions, Drawer, Empty, Input, Progress, Select, Space, Spin, Typography } from 'antd';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { apiGet, apiPost } from '../api.js';
import { openStream } from '../sse.js';
import { StatusTag } from '../ui/statusTag.jsx';
import { TimeText } from '../ui/timeText.jsx';
import { KIND_LABELS, appendMessagePage, executionActions, messagePagePath } from './model.js';
import { TranscriptView } from '../transcript.jsx';
import { turnsFromMessages } from '../reduce.js';
import { useExecutionTranscript } from './detail/liveTranscript.js';
import { Artifacts } from './artifacts.jsx';
import { DetailFields, PayloadWindows } from './detail/fields.jsx';
import { WorkloadDetail, todoInitializationNotice } from './detail/workloads.jsx';
import { BrainRunEmbed } from './detail/brainRun.jsx';
import { TodoRunEmbed } from './detail/todoFiles.jsx';
import { Markdown } from '../project/markdown.jsx';
import { prepareInput } from '../chat/inputAttempt.js';
import { err } from '../notice.js';

const EVENT_TEXT_CHARS = 64 * 1024;
const RETAINED_EVENT_CHARS = 2 * 1024 * 1024;

export function messageRefreshMode(status, windowIndex) {
  if (windowIndex !== 0) return 'paused';
  return ['pending', 'running'].includes(status) ? 'poll' : 'once';
}

function eventText(data) {
  const text = typeof data === 'string' ? data : JSON.stringify(data ?? null);
  return text.length <= EVENT_TEXT_CHARS ? text : `${text.slice(0, EVENT_TEXT_CHARS)}…`;
}

export function ExecutionTranscript({ messages, live }) {
  const turns = useMemo(() => turnsFromMessages(messages), [messages]);
  return <TranscriptView turns={live?.turns || turns} />;
}

export function appendEvent(rows, frame) {
  const next = rows.concat({ seq: frame.seq, event: frame.event, data: frame.data, text: eventText(frame.data) });
  let bytes = next.reduce((sum, event) => sum + event.text.length, 0);
  while (next.length > 1 && (next.length > 200 || bytes > RETAINED_EVENT_CHARS)) {
    bytes -= next.shift().text.length;
  }
  return next;
}

export function ExecutionDetail({ id, summary, onClose, onNotice, managed = false }) {
  // Dispatch-time name (lifted by the list endpoint) reads better than the raw id.
  const title = summary?.name ? `${summary.name} (${id})` : id;
  return <Drawer rootClassName="oc-execution-detail" open={!!id} title={title} onClose={onClose} placement="right" size="75vw" styles={{ wrapper: { maxWidth: '100vw' } }}>
    <ExecutionView key={id} executionRef={{ id, kind: summary?.kind }} summary={summary} onNotice={onNotice} managed={managed} />
  </Drawer>;
}

export function ExecutionView({ executionRef, summary, onNotice, mode = 'full', presentation = 'default', managed = false, allowGuidance = false, onGuidance }) {
  const canEdit = useCanEdit();
  const id = executionRef.id;
  const [childId, setChildId] = useState(null);
  const [loadedDetail, setDetail] = useState(null);
  const detail = loadedDetail?.execution?.id === id ? loadedDetail : null;
  const [error, setError] = useState('');
  const [events, setEvents] = useState([]);
  const [prompt, setPrompt] = useState('');
  const [delivery, setDelivery] = useState('steer');
  const [busy, setBusy] = useState(false);
  const inputAttempt = useRef(null);
  const submitting = useRef(false);
  const [messages, setMessages] = useState({ messages: [], partial: null, nextCursor: null, more: false });
  const [messagesBusy, setMessagesBusy] = useState(false);
  const [messageWindows, setMessageWindows] = useState([{ cursor: null, leading: new Uint8Array() }]);
  const [messageWindow, setMessageWindow] = useState(0);
  const [revision, setRevision] = useState(0);
  const detailRequest = useRef(null);
  const messageRequest = useRef(null);
  const load = useCallback(async () => {
    if (detailRequest.current?.pending && !detailRequest.current.controller.signal.aborted && detailRequest.current.id === id) return;
    detailRequest.current?.controller.abort();
    const controller = new AbortController();
    const request = { id, controller, pending: true }; detailRequest.current = request;
    try {
      const next = await apiGet(`/api/executions/${encodeURIComponent(id)}`, { signal: controller.signal });
      if (!controller.signal.aborted) {
        if (next?.execution?.id !== id) throw new Error('执行详情响应格式错误');
        setDetail(next); setError('');
      }
    } catch (e) {
      if (!controller.signal.aborted) setError(e.status === 503 ? '所属节点当前离线，恢复连接后可读取明细和继续操作' : e.message);
    } finally { request.pending = false; }
  }, [id]);
  useEffect(() => {
    if (!id) return undefined;
    setDetail(null); setEvents([]); setMessages({ messages: [], partial: null, large: [], nextCursor: null, more: false });
    setMessageWindows([{ cursor: null, leading: new Uint8Array() }]); setMessageWindow(0); setError(''); load();
    const timer = setInterval(load, 3000);
    return () => { clearInterval(timer); detailRequest.current?.controller.abort(); messageRequest.current?.abort(); };
  }, [id, load]);
  const index = detail?.execution || summary || null;
  const kind = detail?.request?.kind || index?.kind;
  const detailReady = detail?.execution?.id === id;
  const todoInitialization = kind === 'todos' && !detail?.workflow && todoInitializationNotice(detail);
  const hasMessages = detailReady && ['agent', 'maintenance', 'operator'].includes(kind);
  useEffect(() => {
    if (!detailReady || !id || !kind || ['agent', 'maintenance', 'dag', 'operator'].includes(kind)) return undefined;
    const stream = openStream({ path: `/api/executions/${encodeURIComponent(id)}/events`, after: 0, executionHistory: true,
      onFrame: (frame) => {
        setEvents((rows) => appendEvent(rows, frame));
        if (['done', 'run_finished', 'workflow_completed', 'workflow_failed'].includes(frame.event)) load();
        if (frame.event === 'error') setError(typeof frame.data?.error === 'string' ? frame.data.error : JSON.stringify(frame.data));
      },
      onStatus: (status) => { if (status === 'failed') setError('节点事件流连接失败，可刷新重试'); },
    });
    return () => stream.abort();
  }, [id, kind, detailReady, load]);
  const loadMessages = useCallback(async ({ reset = false, rewind = false, cursor = null, leading = new Uint8Array(), windowIndex = 0 } = {}) => {
    if (!id || !hasMessages) return;
    messageRequest.current?.abort();
    const controller = new AbortController(); messageRequest.current = controller;
    setMessagesBusy(true);
    try {
      const page = await apiGet(messagePagePath(id, cursor), { signal: controller.signal });
      if (controller.signal.aborted) return;
      setMessages((old) => {
        const next = appendMessagePage(reset ? null : { ...old, partial: rewind ? null : old.partial, large: [] }, page, leading);
        return JSON.stringify(next) === JSON.stringify(old) ? old : next;
      });
      setMessageWindow(windowIndex);
    } catch (e) { if (!controller.signal.aborted) setError(e.message); }
    finally { if (!controller.signal.aborted) setMessagesBusy(false); }
  }, [id, kind, hasMessages]);
  const live = useExecutionTranscript({
    id, enabled: detailReady && ['agent', 'maintenance', 'operator'].includes(kind), status: index?.status, revision,
    onFrame: (frame) => setEvents((rows) => appendEvent(rows, frame)),
    onSettled: () => { load(); if (messageWindow === 0) loadMessages({ reset: true }); },
    onError: setError,
  });
  const showLive = messageWindow === 0 && live.caughtUp && ['pending', 'running'].includes(index?.status);
  useEffect(() => {
    if (!hasMessages) return undefined;
    const refresh = messageRefreshMode(index?.status, messageWindow);
    if (refresh === 'paused') return undefined;
    loadMessages({ reset: true });
    if (refresh !== 'poll') return undefined;
    const timer = setInterval(() => loadMessages({ reset: true }), 3000);
    return () => clearInterval(timer);
  }, [id, kind, hasMessages, index?.status, loadMessages, messageWindow]);
  const nextMessages = () => {
    if (!messages.nextCursor) return;
    const lastLarge = messages.large?.[messages.large.length - 1];
    const sameMessage = lastLarge && messages.nextCursor.seq === lastLarge.seq;
    const next = { cursor: messages.nextCursor, leading: sameMessage ? lastLarge.tail : new Uint8Array() };
    const windows = messageWindows.slice(0, messageWindow + 1).concat(next);
    setMessageWindows(windows);
    loadMessages({ cursor: next.cursor, leading: next.leading, windowIndex: windows.length - 1 });
  };
  const previousMessages = () => {
    const index = messageWindow - 1;
    if (index < 0) return;
    const prior = messageWindows[index];
    loadMessages({ rewind: true, cursor: prior.cursor, leading: prior.leading, windowIndex: index });
  };
  const restartMessages = () => {
    setMessageWindows([{ cursor: null, leading: new Uint8Array() }]);
    loadMessages({ reset: true, windowIndex: 0 });
  };
  const command = async (action, input = {}) => {
    if (!canEdit || submitting.current) return;
    submitting.current = true; setBusy(true);
    try {
      if (['prompt', 'steer', 'queue'].includes(action)) {
        const prepared = prepareInput(inputAttempt.current, id, action, input);
        inputAttempt.current = prepared.attempt; input = prepared.input;
      }
      const path = `/api/executions/${encodeURIComponent(id)}/commands`;
      await apiPost(path, { action, input }); inputAttempt.current = null; setPrompt(''); setRevision((v) => v + 1); await load(); }
    catch (e) { onNotice?.(err(e.message)); }
    finally { submitting.current = false; setBusy(false); }
  };
  const submitGuidance = async () => {
    if (!prompt.trim() || !onGuidance) return;
    setBusy(true);
    try { if (await onGuidance(prompt)) setPrompt(''); }
    finally { setBusy(false); }
  };
  const execution = index;
  const actions = executionActions(execution);
  const unavailable = !detail && !!error;
  const process = <>    {hasMessages && <div className="execution-messages">
      <Typography.Title level={5}>会话消息</Typography.Title>
      {!messages.messages.length && !messages.partial && !messagesBusy ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无消息" /> : null}
      {(showLive || !!messages.messages.length) && <ExecutionTranscript messages={messages.messages} live={showLive ? live.state : null} />}
      {showLive && live.state.trimmed && <Alert type="info" title="实时记录仅保留最近的消息，完整历史可在执行结束后分段查看" />}
      {messages.trimmed && <Alert type="info" showIcon title="为保持页面流畅，较早的已加载消息已从当前页面释放" action={<Button size="small" onClick={restartMessages}>从头查看</Button>} />}
      {!!messages.large?.length && <div className="execution-message-large">
        {messages.large.map((large) => <div key={`${large.seq}-${large.start}`}>
          <Alert type="info" showIcon title="这条消息内容较大，已分段显示" description={`当前 ${large.start}–${large.end} / ${large.total} 字节`} />
          <pre>{large.text}</pre>
        </div>)}
        <Space><Button disabled={messageWindow === 0 || messagesBusy} onClick={previousMessages}>上一段</Button><Button disabled={!messages.nextCursor || messagesBusy} onClick={nextMessages}>下一段</Button></Space>
      </div>}
      {messages.partial && <Progress percent={Math.floor((messages.partial.next / Math.max(messages.partial.total, 1)) * 100)} size="small" format={() => `正在读取 ${messages.partial.next} / ${messages.partial.total} 字节`} />}
      {!messages.large?.length && (messages.more || messages.partial) && <Button block loading={messagesBusy} onClick={nextMessages}>继续加载消息</Button>}
      {messagesBusy && !messages.messages.length && !messages.partial ? <Spin size="small" /> : null}
    </div>}
    {kind === 'dag' && (detail?.definition?.spec || detail?.definition)?.steps && <DagRunResult key={id} id={id}
      spec={detail.definition.spec || detail.definition} status={execution?.status}
      onStatus={(status) => { if (status !== execution?.status) load(); }} />}
    {kind !== 'team' && typeof detail?.topic?.final_summary === 'string' && <Markdown text={detail.topic.final_summary} />}
    {/* 过程视图只在完整明细挂载：inline 模式（brain 工作台 Inspector 的「执行过程」页，
        ~380px 窄列）不嵌 PlanCanvas/TODO 画布与第二条 SSE，内联仍由下方 Transcript/
        WorkloadDetail 等轻量块承载过程信息。 */}
    {mode === 'full' && kind === 'brain' && <BrainRunEmbed id={id} onNotice={onNotice} />}
    {mode === 'full' && kind === 'todos' && detailReady && detail.workflow && <TodoRunEmbed id={id} />}
    <WorkloadDetail id={id} detail={detail} kind={kind} onOpen={setChildId} />
</>;
  const guidance = canEdit && allowGuidance && onGuidance && (managed || kind === 'team') && <Space orientation="vertical" style={{ width: '100%', marginTop: 16 }}>
      <Input.TextArea value={prompt} onChange={(e) => setPrompt(e.target.value)} placeholder={managed ? '补充信息，由大脑决定后续调度' : '继续会话'} rows={3} />
      <EditButton type="primary" disabled={!prompt.trim() || busy} loading={busy} onClick={submitGuidance}>提交给大脑</EditButton>
    </Space>;
  const context = <>    {kind === 'dag' && <DagRunContext context={detail?.dag_context} />}
    {kind === 'dag' && <Artifacts id={id} spec={detail?.definition?.spec || detail?.definition} onNotice={onNotice} />}
</>;
  const eventHistory = <>    {kind !== 'dag' && <Collapse style={{ marginTop: 16 }} items={[
      { key: 'events', label: `执行事件（最近 ${events.length} 条）`, children: events.map((e, i) => <div key={`${e.seq}-${i}`}><pre style={{ whiteSpace: 'pre-wrap' }}>#{e.seq} {e.event} {e.data?.omitted ? '内容较大，可分段查看' : e.text}</pre>{e.data?.omitted && e.data?.read_via === 'event_payload' ? <PayloadWindows id={id} marker={e.data} seq={e.seq} label="分段查看事件内容" /> : null}</div>) },
    ]} />}
</>;
  return <div className={`execution-view-${mode}`}>
    {error && <Alert type="error" showIcon title={error} action={<Button aria-label="重试" onClick={load}>重试</Button>} />}
    {!detail && !error && <Spin />}
    {execution && <Descriptions size="small" column={presentation === 'brain' ? { xs: 1, sm: 2 } : 3} items={[
      { key: 'node', label: '所属节点', children: execution.node_id },
      { key: 'kind', label: '类型', children: KIND_LABELS[kind] || kind },
      ...(presentation === 'brain' ? [] : [{ key: 'harness', label: 'Harness', children: detail?.session?.harness || detail?.harness || detail?.request?.input?.harness || 'opencoder' }]),
      { key: 'status', label: '状态', children: <StatusTag status={execution.status} /> },
      { key: 'created', label: '创建时间', children: <TimeText ts={execution.created_at} /> },
    ]} />}
    {!managed && <Space wrap style={{ margin: '12px 0' }}>
      <Button onClick={() => { setRevision((v) => v + 1); load(); }}>刷新明细</Button>
      <Button disabled={!canEdit || busy || unavailable || !actions.resume} onClick={() => command('resume')}>在原节点恢复</Button>
      <Button disabled={!canEdit || busy || unavailable || !actions.interrupt} onClick={() => command('interrupt')}>中断（可恢复）</Button>
      <Button danger disabled={!canEdit || busy || unavailable || !actions.cancel} onClick={() => command('cancel')}>取消（终止）</Button>
    </Space>}
    {presentation !== 'brain' && detail?.error && !todoInitialization && <Alert type="error" title={detail.error} />}
    {presentation !== 'brain' && process}
    {hasMessages && !managed && <Space orientation="vertical" style={{ width: '100%', marginTop: 16 }}>
      {canEdit && <Select aria-label="发送方式" value={delivery} onChange={setDelivery} options={[{ value: 'steer', label: '指导当前执行 / 继续对话' }, { value: 'queue', label: '加入队列' }]} />}
      <ConversationInput value={prompt} onChange={setPrompt} disabled={busy || unavailable} loading={busy}
        placeholder="继续会话" onSubmit={() => { if (prompt.trim()) command(delivery, { prompt }); }} />
      <QueuePanel sessionId={id} refreshSignal={`${revision}-${events[events.length - 1]?.seq || 0}`} />
      {canEdit && <QuestionModal sessionId={id} active={index?.status === 'running'} />}
    </Space>}
    {presentation === 'brain' ? detail && <BrainExecutionPresentation key={id} id={id} detail={detail} process={process} events={eventHistory} guidance={guidance} context={context} />
      : <>{guidance}{context}<DetailFields id={id} detail={detail} />{eventHistory}</>}
    {childId && <ExecutionDetail key={childId} id={childId} onClose={() => setChildId(null)} onNotice={onNotice} managed={managed} />}
  </div>;
}
