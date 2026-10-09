import { useEffect, useRef, useState } from 'react';
import { newId } from '../model.js';
import { acceptanceOf, postExecution, readReceipt, receiptOutcome, RECEIPT_POLL_MS, uncertainFailure } from './submission.js';

const empty = { busy: false, error: '', confirming: false, attempt: null };
export function useTeamSubmission(team, onAccepted) {
  const [state, setState] = useState(empty);
  const attempt = useRef(null), sending = useRef(false), active = useRef(null), accepted = useRef(onAccepted);
  accepted.current = onAccepted;
  const name = team?.name;
  useEffect(() => {
    const controller = new AbortController();
    active.current = controller;
    sending.current = false;
    if (!name || !attempt.current || attempt.current.request.target !== name) setState(empty);
    else setState((old) => ({ ...old, busy: false, confirming: true, attempt: attempt.current }));
    return () => controller.abort();
  }, [name]);
  const complete = (result, controller) => {
    if (controller.signal.aborted) return;
    attempt.current = null;
    setState(empty);
    accepted.current({ ...result, name, kind: 'team' });
  };
  useEffect(() => {
    if (!name || !state.confirming || state.attempt?.request.target !== name) return;
    const controller = new AbortController();
    let timer;
    const check = async () => {
      try {
        const result = receiptOutcome(await readReceipt(state.attempt.id, controller.signal), state.attempt.id);
        if (controller.signal.aborted) return;
        if (result) { complete(result, controller); return; }
        setState((old) => ({ ...old, error: '', registered: true }));
      } catch (error) {
        if (controller.signal.aborted) return;
        if (error.rejected) {
          attempt.current = null;
          setState({ ...empty, error: error.message });
          return;
        }
        setState((old) => ({ ...old, error: error.status === 404 ? '' : `确认启动结果失败：${error.message}` }));
      }
      timer = setTimeout(check, RECEIPT_POLL_MS);
    };
    check();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [name, state.confirming, state.attempt]);
  const submit = async (values) => {
    if (!name || sending.current) return;
    const controller = active.current;
    sending.current = true;
    setState((old) => ({ ...old, busy: true, confirming: false, error: '' }));
    try {
      const input = { kind: 'team', target: name, node_id: values.node || null, input: { prompt: values.prompt.trim() } };
      const signature = JSON.stringify(input);
      if (attempt.current?.request.target === name && attempt.current.signature !== signature) throw new Error('上次提交尚未确认，请先确认原请求');
      if (!attempt.current || attempt.current.request.target !== name) {
        const id = newId('team');
        attempt.current = { id, signature, request: { ...input, id } };
      }
      const current = attempt.current;
      setState((old) => ({ ...old, attempt: current }));
      complete(acceptanceOf(await postExecution(current.request, controller.signal), current.id), controller);
    } catch (error) {
      if (controller.signal.aborted) return;
      const confirming = !!attempt.current && uncertainFailure(error);
      if (!confirming) attempt.current = null;
      setState({ busy: false, error: confirming ? '' : error.message, confirming, attempt: attempt.current });
    } finally {
      if (!controller.signal.aborted) { sending.current = false; setState((old) => ({ ...old, busy: false })); }
    }
  };
  return { ...state, submit };
}
