// Session event subscription shared by prompt submission and compaction.
import { useCallback } from 'react';
import { apiGet } from '../api.js';
import { openStream } from '../sse.js';
import { emptyStream, ensurePendingEcho, reduceFrame, resyncState, turnsFromMessages, usageFromMessages } from '../reduce.js';
import { err } from '../notice.js';

export function useTranscriptStream({ streamRef, aliveRef, setStream, setBusy, setConnecting, setQueueVersion, onNotice, selectionRef }) {
  /// Normalize the transcript from the store once a run reaches `done` —
  /// mirrors the vanilla frontend's done → loadTranscript(). Kept best-effort:
  /// if the fetch fails or returns nothing we keep the streamed turns.
  const reloadAfterDone = useCallback(async (sid) => {
    const owner = selectionRef.current.node;
    const current = () => aliveRef.current && selectionRef.current.node === owner && selectionRef.current.dialog === sid;
    try {
      const j = await apiGet('/api/sessions/' + encodeURIComponent(sid));
      const msgs = (j && j.messages) || [];
      if (msgs.length && current()) {
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
    } catch (e) {
      if (current()) onNotice?.(err('刷新会话记录失败: ' + e.message));
    }
  }, [onNotice, selectionRef, aliveRef, setStream]);

  const startStream = useCallback(({ path, sessionId, after, initialTurns, initialUsage }) => {
    const owner = selectionRef.current.node;
    const current = () => aliveRef.current && selectionRef.current.node === owner && selectionRef.current.dialog === sessionId;
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
        if (!current()) return;
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
      onStatus: (st) => {
        if (!current()) return;
        if (st === 'failed') {
          setConnecting(false);
          setBusy(false);
          if (onNotice) {
            onNotice(err('SSE 流连接失败（已重试 5 次）'));
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
          if (!current()) {
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
  }, [onNotice, reloadAfterDone, selectionRef, aliveRef, setStream, setBusy, setConnecting, setQueueVersion, streamRef]);

  /// seq head → authenticated /events stream (prompt + 压缩 share the open path).
  /// `after` is the pre-POST seq head owned by the caller; when omitted we
  /// fetch /seq here (no ordering guarantee —
  /// callers that need only-this-turn's events must snapshot BEFORE posting).
  /// `initialTurns` threads the caller's optimistic echo turns into
  /// startStream's reset state (see startStream's comment).
  const openSessionStream = useCallback(async (sid, after, initialTurns) => {
    let head = after;
    if (head === undefined) {
      const q = await apiGet('/api/sessions/' + encodeURIComponent(sid) + '/seq');
      head = q?.seq || 0;
    }
    startStream({
      path: '/api/sessions/' + encodeURIComponent(sid) + '/events',
      sessionId: sid,
      after: head,
      initialTurns,
    });
  }, [startStream]);

  return { reloadAfterDone, startStream, openSessionStream };
}
