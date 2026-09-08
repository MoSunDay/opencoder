import { RunReplay } from '../project/replay/run.jsx';
import { submitAttempt } from '../project/replay/attempt.js';
import { Alert, Button, Collapse, Descriptions, Drawer, Empty, Input, Progress, Select, Space, Spin, Typography } from 'antd';
import { useCallback, useEffect, useMemo, useState } from 'react';
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
import { WorkloadDetail } from './detail/workloads.jsx';
import { Markdown } from '../project/markdown.jsx';
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

export function ExecutionDetail({ id, summary, onClose, onNotice }) {
  const [childId, setChildId] = useState(null);
  const [detail, setDetail] = useState(null);
  const [error, setError] = useState('');
  const [events, setEvents] = useState([]);
  const [prompt, setPrompt] = useState('');
  const [delivery, setDelivery] = useState('prompt');
  const [busy, setBusy] = useState(false);
  const [messages, setMessages] = useState({ messages: [], partial: null, nextCursor: null, more: false });
  const [messagesBusy, setMessagesBusy] = useState(false);
  const [messageWindows, setMessageWindows] = useState([{ cursor: null, leading: new Uint8Array() }]);
  const [messageWindow, setMessageWindow] = useState(0);
  const [revision, setRevision] = useState(0);
  const load = useCallback(async () => {
    try { setDetail(await apiGet(`/api/executions/${encodeURIComponent(id)}`)); setError(''); }
    catch (e) { setDetail(null); setError(e.status === 503 ? '所属节点当前离线，恢复连接后可读取明细和继续操作' : e.message); }
  }, [id]);
  useEffect(() => {
    if (!id) return undefined;
    setDetail(null); setEvents([]); setMessages({ messages: [], partial: null, large: [], nextCursor: null, more: false });
    setMessageWindows([{ cursor: null, leading: new Uint8Array() }]); setMessageWindow(0); setError(''); load();
    const timer = setInterval(load, 3000);
    return () => clearInterval(timer);
  }, [id, load]);
  const index = detail?.execution || summary || null;
  const kind = detail?.request?.kind || index?.kind;
  const isProjectRun = kind === 'project' && id.startsWith('prun-');
  const hasMessages = ['agent', 'maintenance'].includes(kind) || (isProjectRun && !!detail?.run?.session_id);
  useEffect(() => {
    if (!id || !kind || isProjectRun || ['agent', 'maintenance'].includes(kind)) return undefined;
    const stream = openStream({ path: `/api/executions/${encodeURIComponent(id)}/events`, after: 0, executionHistory: true,
      onFrame: (frame) => {
        setEvents((rows) => appendEvent(rows, frame));
        if (['done', 'run_finished', 'workflow_completed', 'workflow_failed'].includes(frame.event)) load();
        if (frame.event === 'error') setError(typeof frame.data?.error === 'string' ? frame.data.error : JSON.stringify(frame.data));
      },
      onStatus: (status) => { if (status === 'failed') setError('节点事件流连接失败，可刷新重试'); },
    });
    return () => stream.abort();
  }, [id, kind, isProjectRun, load]);
  const loadMessages = useCallback(async ({ reset = false, rewind = false, cursor = null, leading = new Uint8Array(), windowIndex = 0 } = {}) => {
    if (!id || !hasMessages) return;
    setMessagesBusy(true);
    try {
      const page = await apiGet(messagePagePath(id, cursor));
      setMessages((old) => {
        const next = appendMessagePage(reset ? null : { ...old, partial: rewind ? null : old.partial, large: [] }, page, leading);
        return JSON.stringify(next) === JSON.stringify(old) ? old : next;
      });
      setMessageWindow(windowIndex);
    } catch (e) { setError(e.message); }
    finally { setMessagesBusy(false); }
  }, [id, kind, hasMessages]);
  const live = useExecutionTranscript({
    id, enabled: ['agent', 'maintenance'].includes(kind), status: index?.status, revision,
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
    const lastLarge = messages.large?.at(-1);
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
    setBusy(true);
    try {
      const path = `/api/executions/${encodeURIComponent(id)}/commands`;
      if (kind === 'project' && ['plan', 'execute'].includes(action)) await submitAttempt(id.slice(8), action, input, path);
      else await apiPost(path, { action, input }); setPrompt(''); setRevision((v) => v + 1); await load(); }
    catch (e) { onNotice(err(e.message)); }
    finally { setBusy(false); }
  };
  const execution = index;
  const projectRunnable = ['idle', 'interrupted', 'error'].includes(execution?.status);
  const actions = isProjectRun ? {} : executionActions(execution);
  const unavailable = !detail && !!error;
  return <Drawer open={!!id} title={id} onClose={onClose} placement="right" size="75vw" styles={{ wrapper: { maxWidth: '100vw' } }}>
    {error && <Alert type="error" showIcon title={error} />}
    {execution && <Descriptions size="small" items={[
      { key: 'node', label: '所属节点', children: execution.node_id },
      { key: 'kind', label: '类型', children: KIND_LABELS[kind] || kind },
      { key: 'status', label: '状态', children: <StatusTag status={execution.status} /> },
      { key: 'created', label: '创建时间', children: <TimeText ts={execution.created_at} /> },
    ]} />}
    <Space wrap style={{ margin: '12px 0' }}>
      <Button onClick={() => { setRevision((v) => v + 1); load(); }}>刷新明细</Button>
      <Button disabled={busy || unavailable || !actions.resume} onClick={() => command('resume')}>在原节点恢复</Button>
      <Button disabled={busy || unavailable || !actions.interrupt} onClick={() => command('interrupt')}>中断（可恢复）</Button>
      <Button danger disabled={busy || unavailable || !actions.cancel} onClick={() => command('cancel')}>取消（终止）</Button>
      {kind === 'project' && !isProjectRun && <><Button disabled={busy || unavailable || !projectRunnable} onClick={() => command('plan')}>生成计划</Button><Button disabled={busy || unavailable || !projectRunnable || !detail?.todo?.plan_md} onClick={() => command('execute')}>执行计划</Button></>}
    </Space>
    {detail?.error && <Alert type="error" title={detail.error} />}
    {hasMessages && <div className="execution-messages">
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
    {['agent', 'maintenance'].includes(kind) && <Space orientation="vertical" style={{ width: '100%', marginTop: 16 }}>
      <Input.TextArea disabled={unavailable} value={prompt} onChange={(e) => setPrompt(e.target.value)} placeholder="继续会话" rows={3} />
      <Space><Select value={delivery} onChange={setDelivery} options={[{ value: 'prompt', label: '发送' }, { value: 'steer', label: '指导当前执行' }, { value: 'queue', label: '加入队列' }]} /><Button type="primary" disabled={!prompt.trim()} loading={busy} onClick={() => command(delivery, { prompt })}>提交</Button></Space>
    </Space>}
    {kind === 'dag' && <Artifacts id={id} spec={detail?.definition?.spec || detail?.definition} onNotice={onNotice} />}
    {detail?.topic?.final_summary && <Markdown text={detail.topic.final_summary} />}
    <WorkloadDetail id={id} detail={detail} kind={kind} onOpen={setChildId} />
    {isProjectRun && detail?.run && <RunReplay id={id} detail={detail} onOpen={setChildId} />}
    <DetailFields id={id} detail={detail} />
    {!isProjectRun && <Collapse style={{ marginTop: 16 }} items={[
      { key: 'events', label: `执行事件（最近 ${events.length} 条）`, children: events.map((e, i) => <div key={`${e.seq}-${i}`}><pre style={{ whiteSpace: 'pre-wrap' }}>#{e.seq} {e.event} {e.data?.omitted ? '内容较大，可分段查看' : e.text}</pre>{e.data?.omitted && e.data?.read_via === 'event_payload' ? <PayloadWindows id={id} marker={e.data} seq={e.seq} label="分段查看事件内容" /> : null}</div>) },
    ]} />}
    {childId && <ExecutionDetail key={childId} id={childId} onClose={() => setChildId(null)} onNotice={onNotice} />}
  </Drawer>;
}
