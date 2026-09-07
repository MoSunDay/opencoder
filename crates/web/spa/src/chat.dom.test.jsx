// @vitest-environment jsdom
// Real composer, authenticated requests and streamed transcript boundaries.
import { describe, expect, it } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';

import './test/setup-dom.js';
import { ChatPanel } from './chat.jsx';
import { TranscriptView } from './transcript.jsx';
import { setCredentials, setState } from './store.js';

import { chatTestState } from './chat/testSupport.js';

const turnsFixture = () => [
  { kind: 'text', role: 'user', text: '帮我跑一遍测试' },
  { kind: 'text', role: 'assistant', text: '好的，开始执行' },
  { kind: 'think', role: 'assistant', text: '先理解需求…' },
  { kind: 'tool', role: 'assistant', name: 'bash', input: 'npm test', output: 'Tests 32 passed', isError: true, durationMs: 1200, open: true },
  { kind: 'sys', text: 'status: streaming' },
];

describe('TranscriptView on Bubble.List', () => {
  it('renders turns as bubbles with tool error tag and usage footer', () => {
    const { container } = render(
      <TranscriptView
        turns={turnsFixture()}
        usage={{ input: 10, output: 5, total: 15, contextWindow: 100 }}
        status="streaming"
        error={null}
      />,
    );
    expect(container.querySelector('.ant-bubble-list')).toBeTruthy();
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(5);
    expect(container.querySelector('.ant-bubble-end')).toBeTruthy();
    expect(container.querySelectorAll('.ant-bubble-start')).toHaveLength(4);
    expect(screen.getByText('error')).toBeTruthy();
    expect(screen.getByText(/🔧 bash/)).toBeTruthy();
    expect(screen.getByText(/▲ in 10/)).toBeTruthy();
    expect(screen.getByText(/上下文 15%/)).toBeTruthy();
    expect(screen.getByText('streaming…')).toBeTruthy();
  });
});

describe('ChatPanel full chain (Sender → signed POST → SSE)', () => {
  const mountChat = async () => {
    setCredentials('smoke-token', '');
    const { container } = render(<ChatPanel />);
    const textarea = await waitFor(() => {
      const el = container.querySelector('textarea.ant-sender-input');
      expect(el).toBeTruthy();
      expect(el.disabled).toBe(false);
      return el;
    });
    return { container, textarea };
  };

  it('posts the typed prompt on Enter and clears the controlled input', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '你好，帮我跑个测试' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      const promptHit = chatTestState.hits.find((h) => h.url.includes('/prompt'));
      expect(promptHit).toBeTruthy();
      expect(JSON.parse(promptHit.body).prompt).toBe('你好，帮我跑个测试');
    });
    await waitFor(() => {
      expect(container.querySelector('textarea.ant-sender-input').value).toBe('');
    });
    expect(JSON.parse(chatTestState.hits.find((hit) => hit.method === 'POST' && hit.url === '/api/sessions').body).node_id).toBe('node-1');
    expect(chatTestState.hits.some((h) => h.method === 'POST' && h.url === '/api/sessions')).toBe(true);
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.url.includes('/api/sessions/s1/events'))).toBe(true);
    });
  });

  it('snapshots the /seq head BEFORE posting the prompt (lost-frame race)', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: 'race' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.url.includes('/api/sessions/s1/events'))).toBe(true);
    });
    const promptIdx = chatTestState.hits.findIndex((h) => h.method === 'POST' && h.url.includes('/prompt'));
    const seqIdx = chatTestState.hits.findIndex((h) => h.method === 'GET' && /\/api\/sessions\/[^/]+\/seq/.test(h.url));
    const eventsIdx = chatTestState.hits.findIndex((h) => h.url.includes('/api/sessions/s1/events'));
    expect(promptIdx).toBeGreaterThan(-1);
    expect(seqIdx).toBeGreaterThan(-1);
    expect(seqIdx).toBeLessThan(promptIdx);
    expect(eventsIdx).toBeGreaterThan(promptIdx);
  });

  it('swaps in the Sender stop button while busy and posts interrupt on click', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '长任务' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    const stop = await waitFor(() => {
      const el = container.querySelector('.ant-sender-actions-btn-loading-button');
      expect(el).toBeTruthy();
      expect(el.disabled).toBe(false);
      return el;
    });
    await act(async () => {
      fireEvent.click(stop);
    });
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.method === 'POST' && h.url.includes('/interrupt'))).toBe(true);
    });
  });

  it('releases the composer on a terminal error frame (busy must not latch)', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '会失败的任务' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeTruthy();
    });
    await waitFor(() => {
      expect(chatTestState.liveEventCtl).toBeTruthy();
    });
    const enc = new TextEncoder();
    await act(async () => {
      chatTestState.liveEventCtl.enqueue(enc.encode(
        'event: error\ndata: ' + JSON.stringify({ error: 'boom' }) + '\n\n',
      ));
    });
    await waitFor(() => {
      expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeFalsy();
    });
  });

  it('keeps the typed input when a remote run is already busy', async () => {
    setState({ page: 'chat', preselectNode: 'node-1', nodes: [], conn: 'init' });
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '第一个远程任务' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.url === '/api/sessions' && h.method === 'POST')).toBe(true);
    });
    expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeTruthy();

    await act(async () => {
      fireEvent.change(textarea, { target: { value: '第二个输入不能丢' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(container.querySelector('textarea.ant-sender-input').value).toBe('第二个输入不能丢');
    });
    expect(chatTestState.hits.filter((h) => h.url === '/api/sessions' && h.method === 'POST')).toHaveLength(1);
  });

  it('releases the composer when a FIRST remote dispatch reaches a terminal frame (no dialog selected yet)', async () => {
    setState({ page: 'chat', preselectNode: 'node-1', nodes: [], conn: 'init' });
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '首个远程任务' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.url === '/api/sessions' && h.method === 'POST')).toBe(true);
    });
    expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeTruthy();

    const enc = new TextEncoder();
    await act(async () => {
      chatTestState.liveEventCtl.enqueue(enc.encode('event: done\ndata: {}\n\n'));
    });
    await waitFor(() => {
      expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeFalsy();
    });
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.url === '/api/sessions/s1')).toBe(true);
    });
  });
});

describe('ChatPanel optimistic echo & transcript_reset rebuild', () => {
  const mountChat = async () => {
    setCredentials('smoke-token', '');
    const { container } = render(<ChatPanel />);
    const textarea = await waitFor(() => {
      const el = container.querySelector('textarea.ant-sender-input');
      expect(el).toBeTruthy();
      expect(el.disabled).toBe(false);
      return el;
    });
    return { container, textarea };
  };

  it('renders the optimistic user bubble right after a fresh local submit (no frames yet)', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '马上开始' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      const bubble = container.querySelector('.ant-bubble-end');
      expect(bubble).toBeTruthy();
      expect(bubble.textContent).toContain('马上开始');
    });
    expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBeTruthy();
  });

  it('renders no optimistic bubble for a bare control command submit', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '/act' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.url.includes('/events'))).toBe(true);
    });
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(0);
  });

  it('re-pushes the pending echo after transcript_reset when the store snapshot lacks it', async () => {
    chatTestState.sessionSnapshots.s1 = {
      messages: [{ role: 'assistant', blocks: [{ kind: 'text', text: '压缩后的上下文' }] }],
    };
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '/act_clear_context 收尾总结' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      const bubble = container.querySelector('.ant-bubble-end');
      expect(bubble).toBeTruthy();
      expect(bubble.textContent).toContain('收尾总结');
    });
    await waitFor(() => {
      expect(chatTestState.liveEventCtl).toBeTruthy();
    });
    const enc = new TextEncoder();
    await act(async () => {
      chatTestState.liveEventCtl.enqueue(enc.encode('event: transcript_reset\ndata: {}\n\n'));
    });
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.url === '/api/sessions/s1')).toBe(true);
    });
    await waitFor(() => {
      const ends = container.querySelectorAll('.ant-bubble-end');
      expect(ends).toHaveLength(1);
      expect(ends[0].textContent).toContain('收尾总结');
    });
    expect(screen.getByText('压缩后的上下文')).toBeTruthy();
    expect(container.querySelectorAll('.ant-bubble')).toHaveLength(2);
  });

  it('dedups the server echo frame against the optimistic bubble (one user bubble live)', async () => {
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '马上开始' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(chatTestState.liveEventCtl).toBeTruthy();
      expect(container.querySelector('.ant-bubble-end')).toBeTruthy();
    });
    const enc = new TextEncoder();
    await act(async () => {
      chatTestState.liveEventCtl.enqueue(enc.encode('event: steer_consumed\ndata: {"text":"马上开始"}\n\n'));
    });
    await waitFor(() => {
      expect(container.querySelectorAll('.ant-bubble-end')).toHaveLength(1);
    });
    expect(container.querySelectorAll('.ant-bubble-end')[0].textContent).toContain('马上开始');
    await act(async () => {
      chatTestState.liveEventCtl.enqueue(enc.encode('event: steer_consumed\ndata: {"text":"换个方向"}\n\n'));
    });
    await waitFor(() => {
      expect(container.querySelectorAll('.ant-bubble-end')).toHaveLength(2);
    });
  });
});

describe('ChatPanel resync (lag → snapshot rebuild at the /seq watermark)', () => {
  const mountChat = async () => {
    setCredentials('smoke-token', '');
    const { container } = render(<ChatPanel />);
    const textarea = await waitFor(() => {
      const el = container.querySelector('textarea.ant-sender-input');
      expect(el).toBeTruthy();
      expect(el.disabled).toBe(false);
      return el;
    });
    return { container, textarea };
  };

  it('rebuilds from the snapshot on lag, drops at/below-watermark frames, keeps the live tail', async () => {
    chatTestState.seqHead = 30;
    chatTestState.sessionSnapshots['s1'] = {
      draining: true,
      messages: [
        { role: 'user', blocks: [{ type: 'text', text: '帮我跑测试' }] },
        { role: 'assistant', blocks: [{ type: 'text', text: '快照真相' }] },
      ],
    };
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '继续' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.url.includes('/api/sessions/s1/events'))).toBe(true);
    });
    const enc = new TextEncoder();
    await act(async () => {
      chatTestState.liveEventCtl.enqueue(enc.encode('event: text_delta\ndata: {"text":"局部"}\n\n'));
    });
    await waitFor(() => {
      expect(screen.getByText('局部')).toBeTruthy();
    });
    await act(async () => {
      chatTestState.liveEventCtl.enqueue(enc.encode('event: error\ndata: {"error":"event lag: 5 events dropped","lag":5}\n\n'));
    });
    await new Promise((r) => setTimeout(r, 1200));
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.method === 'GET' && h.url.includes('/api/sessions/s1/events?after=30'))).toBe(true);
    });
    await waitFor(() => {
      expect(screen.getByText('快照真相')).toBeTruthy();
    });
    expect(screen.queryByText('局部')).toBe(null);
    await act(async () => {
      chatTestState.liveEventCtl.enqueue(enc.encode('event: text_delta\nid: 12\ndata: {"text":"旧帧"}\n\n'));
      chatTestState.liveEventCtl.enqueue(enc.encode('event: text_delta\ndata: {"text":"尾部"}\n\n'));
    });
    await waitFor(() => {
      expect(screen.getByText('尾部')).toBeTruthy();
    });
    expect(screen.queryByText('旧帧')).toBe(null);
  }, 15000);

  it('a run that finished while disconnected lands done (draining flag), not latched streaming', async () => {
    chatTestState.seqHead = 30;
    chatTestState.sessionSnapshots['s1'] = {
      draining: false,
      messages: [
        { role: 'user', blocks: [{ type: 'text', text: '收尾' }] },
        { role: 'assistant', blocks: [{ type: 'text', text: '终局快照' }] },
      ],
    };
    const { container, textarea } = await mountChat();
    await act(async () => {
      fireEvent.change(textarea, { target: { value: '继续' } });
    });
    await act(async () => {
      fireEvent.keyDown(container.querySelector('textarea.ant-sender-input'), { key: 'Enter', keyCode: 13 });
    });
    await waitFor(() => {
      expect(chatTestState.hits.some((h) => h.url.includes('/api/sessions/s1/events'))).toBe(true);
    });
    const enc = new TextEncoder();
    await act(async () => {
      chatTestState.liveEventCtl.enqueue(enc.encode('event: error\ndata: {"error":"event lag: 2 events dropped","lag":2}\n\n'));
    });
    await new Promise((r) => setTimeout(r, 1200));
    await waitFor(() => {
      expect(screen.getByText('终局快照')).toBeTruthy();
    });
    await waitFor(() => {
      expect(screen.queryByText('streaming…')).toBe(null);
    });
    await waitFor(() => {
      expect(container.querySelector('.ant-sender-actions-btn-loading-button')).toBe(null);
    });
  }, 15000);
});
