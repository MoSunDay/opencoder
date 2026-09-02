// @vitest-environment jsdom
// QuestionModal DOM smoke: the 2s poll loop only runs while active, an open
// question renders as an un-dismissable modal whose option buttons / free
// text / 跳过 hit the answer & skip endpoints, and every resolve re-polls
// immediately. Fake timers drive the interval; api.js is module-mocked.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';

const { apiGetMock, apiPostMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
}));
vi.mock('./api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
}));

import './test/setup-dom.js';
import { openQuestionOf, QuestionModal } from './questionModal.jsx';

const QUESTION = { id: 'call-1', question: '继续执行吗？', options: ['继续', '停下'] };

beforeEach(() => {
  vi.useFakeTimers();
  apiGetMock.mockReset();
  apiPostMock.mockReset().mockResolvedValue({ ok: true });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/// Flush pending promises so the in-flight poll lands before assertions.
const flush = () => act(async () => {
  await Promise.resolve();
  await Promise.resolve();
});

describe('openQuestionOf', () => {
  it('picks the first well-formed question, tolerates garbage', () => {
    expect(openQuestionOf(undefined)).toBeNull();
    expect(openQuestionOf({})).toBeNull();
    expect(openQuestionOf({ questions: 'nope' })).toBeNull();
    expect(openQuestionOf({ questions: [null, QUESTION] })).toEqual(QUESTION);
    expect(openQuestionOf({ questions: [{ id: 'bare' }] })).toEqual({ id: 'bare' });
  });
});

describe('QuestionModal', () => {
  it('stays out of the way when inactive or no question is waiting', async () => {
    apiGetMock.mockResolvedValue({ questions: [] });
    const { container } = render(<QuestionModal sessionId="s1" active={false} />);
    await flush();
    expect(apiGetMock).not.toHaveBeenCalled();
    expect(container.textContent).toBe('');
    // Active with an empty queue: polls, renders nothing.
    render(<QuestionModal sessionId="s1" active={true} />);
    await flush();
    expect(apiGetMock).toHaveBeenCalledTimes(1);
    expect(screen.queryByText('模型提问')).toBeNull();
  });

  it('polls immediately on activation and again on the 2s tick', async () => {
    apiGetMock.mockResolvedValue({ questions: [] });
    render(<QuestionModal sessionId="s1" active={true} />);
    await flush();
    expect(apiGetMock).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(apiGetMock).toHaveBeenCalledTimes(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(apiGetMock).toHaveBeenCalledTimes(3);
    expect(apiGetMock).toHaveBeenCalledWith('/api/sessions/s1/questions');
  });

  it('renders an open question un-dismissable; Esc does not close it', async () => {
    apiGetMock.mockResolvedValue({ questions: [QUESTION] });
    render(<QuestionModal sessionId="s1" active={true} />);
    await flush(); // findBy* would hang under fake timers — flush + getBy
    expect(screen.getByText('继续执行吗？')).toBeTruthy();
    expect(screen.getByRole('button', { name: '继 续' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '停 下' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '提 交' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '跳 过' })).toBeTruthy();
    // No ✕, no cancel, no mask click, no Esc — the exits are answer/skip.
    expect(document.querySelector('.ant-modal-close')).toBeNull();
    fireEvent.keyDown(document, { key: 'Escape', keyCode: 27 });
    expect(screen.getByText('继续执行吗？')).toBeTruthy();
  });

  it('answers with the clicked option text, then re-polls right away', async () => {
    apiGetMock.mockResolvedValue({ questions: [QUESTION] });
    render(<QuestionModal sessionId="s1" active={true} />);
    await flush(); // findBy* would hang under fake timers — flush + getBy
    expect(screen.getByText('继续执行吗？')).toBeTruthy();
    const calls = apiGetMock.mock.calls.length;
    fireEvent.click(screen.getByRole('button', { name: '继 续' }));
    await flush();
    expect(apiPostMock).toHaveBeenCalledWith('/api/sessions/s1/questions/call-1/answer', { answer: '继续' });
    // The immediate re-poll — not the 2s tick — refreshes the queue.
    expect(apiGetMock.mock.calls.length).toBe(calls + 1);
  });

  it('submits the free-text answer through the 提交 button', async () => {
    apiGetMock.mockResolvedValue({ questions: [QUESTION] });
    render(<QuestionModal sessionId="s1" active={true} />);
    await flush(); // findBy* would hang under fake timers — flush + getBy
    expect(screen.getByText('继续执行吗？')).toBeTruthy();
    fireEvent.change(screen.getByPlaceholderText('或输入自定义回答…'), { target: { value: '换个思路' } });
    fireEvent.click(screen.getByRole('button', { name: '提 交' }));
    await flush();
    expect(apiPostMock).toHaveBeenCalledWith('/api/sessions/s1/questions/call-1/answer', { answer: '换个思路' });
  });

  it('skip posts the skip endpoint and closes the modal when none remains', async () => {
    apiGetMock.mockResolvedValueOnce({ questions: [QUESTION] });
    apiGetMock.mockResolvedValue({ questions: [] });
    render(<QuestionModal sessionId="s1" active={true} />);
    await flush(); // findBy* would hang under fake timers — flush + getBy
    expect(screen.getByText('继续执行吗？')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '跳 过' }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });
    expect(apiPostMock).toHaveBeenCalledWith('/api/sessions/s1/questions/call-1/skip', {});
    // The queue was empty on the re-poll (2nd call) → the question cleared.
    // jsdom never fires transitionend, so the exit motion cannot finish —
    // assert the leave phase instead of unmount.
    expect(apiGetMock.mock.calls.length).toBe(2);
    expect(document.querySelector('.ant-modal.ant-zoom-leave')).toBeTruthy();
  });
});
