// questionModal.jsx — poll bridge for the `question` tool (TUI parity for
// non-SSE-driven flows: the tool blocks on a hub the poll itself attaches,
// crates/web/src/api_questions.rs). Polling runs ONLY while `active` — chat.jsx
// passes busy (the live stream is running) — so a transcript replay never
// resurrects a stale question. Endpoints:
//   GET  /api/sessions/:id/questions            → {questions:[{id,question,options}]}
//   POST /api/sessions/:id/questions/:id/answer {answer}
//   POST /api/sessions/:id/questions/:id/skip
// The modal is deliberately un-dismissable (no ✕, no mask click, no Esc):
// the only exits are answering or skipping, exactly like the TUI prompt.

import { Button, Input, Modal, Typography } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet, apiPost } from './api.js';

const { Text, Paragraph } = Typography;

const POLL_MS = 2000;

/// The question to render: the first entry with a question body. Accepts a
/// raw poll response ({questions} missing on {} fallbacks / 404) — pure so
/// vitest pins the selection rule without a DOM.
export function openQuestionOf(pollState) {
  const st = pollState || {};
  const list = Array.isArray(st.questions) ? st.questions : [];
  return list.find((q) => q && (q.question || q.id)) || null;
}

export function QuestionModal({ sessionId, active }) {
  const [question, setQuestion] = useState(null);
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const aliveRef = useRef(true);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  const poll = useCallback(async () => {
    if (!sessionId) {
      return;
    }
    try {
      const j = await apiGet('/api/sessions/' + encodeURIComponent(sessionId) + '/questions');
      if (aliveRef.current) {
        setQuestion(openQuestionOf(j));
      }
    } catch {
      // 404 (session gone) or transient failure — keep quiet, retry on tick.
    }
  }, [sessionId]);

  // Poll immediately on activation, then every POLL_MS while active. The
  // interval closure reads the latest `poll` (sessionId change re-arms it);
  // deactivation clears the timer AND any rendered question.
  useEffect(() => {
    if (!active || !sessionId) {
      setQuestion(null);
      setText('');
      return undefined;
    }
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => {
      clearInterval(timer);
    };
  }, [active, sessionId, poll]);

  const resolve = useCallback(async (action, body) => {
    if (!question || !sessionId) {
      return;
    }
    setBusy(true);
    try {
      await apiPost(
        '/api/sessions/' + encodeURIComponent(sessionId)
          + '/questions/' + encodeURIComponent(question.id) + '/' + action,
        body,
      );
    } catch {
      // 404 = already resolved elsewhere — the re-poll below converges.
    }
    setBusy(false);
    setText('');
    setQuestion(null);
    poll(); // another question may be waiting behind this one
  }, [question, sessionId, poll]);

  const answer = (value) => resolve('answer', { answer: String(value || '') });
  const skip = () => resolve('skip', {});

  return (
    <Modal
      title="模型提问"
      open={!!question}
      closable={false}
      mask={{ closable: false }}
      keyboard={false}
      footer={null}
      destroyOnHidden
    >
      {question ? (
        <div>
          <Paragraph style={{ marginBottom: 12, whiteSpace: 'pre-wrap' }}>{question.question || ''}</Paragraph>
          {(question.options || []).map((opt) => (
            <Button
              key={opt}
              block
              disabled={busy}
              style={{ marginBottom: 8, textAlign: 'left' }}
              onClick={() => answer(opt)}
            >
              {opt}
            </Button>
          ))}
          <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
            <Input
              value={text}
              disabled={busy}
              placeholder="或输入自定义回答…"
              onChange={(e) => setText(e.target.value)}
              onPressEnter={() => text.trim() && answer(text.trim())}
            />
            <Button type="primary" disabled={busy || !text.trim()} onClick={() => answer(text.trim())}>提交</Button>
            <Button disabled={busy} onClick={skip}>跳过</Button>
          </div>
          <Text type="secondary" style={{ fontSize: 12 }}>回答或跳过后才会继续执行</Text>
        </div>
      ) : null}
    </Modal>
  );
}
