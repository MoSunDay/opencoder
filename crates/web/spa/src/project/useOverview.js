// useOverview.js — the project module's single overview source (iteration 4
// extraction from project.jsx, behavior-identical): owns the
// /api/project/overview state + adaptive polling — 3s while any todo is
// running, else 8s, the interval re-arming whenever the busy flag flips —
// so 进展 / Owner 视角 ride the exact same snapshot and cadence as 项目
// without each panel re-implementing the loop. Derivations stay pure and
// exported (overviewBusy) for panels and tests alike.

import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../api.js';
import { flattenTodos } from './todosTab.jsx';

const POLL_BUSY_MS = 3000;
const POLL_IDLE_MS = 8000;

/// True when any todo row (milestone or backlog) is running — the adaptive
/// poll's fast-cadence flag. Pure over the overview snapshot.
export function overviewBusy(overview) {
  return flattenTodos(overview).some((t) => t.status === 'running');
}

/// useOverview({ onNotice }) → { overview, loading, busy, refresh }
/// - overview: latest snapshot; null before the first load lands, and the
///   canonical `{ goals: [], backlog: [] }` shape for empty payloads.
/// - loading: true only during the loud (non-silent) load — panels wrap
///   their body in <Spin spinning={loading}>.
/// - busy: overviewBusy(overview); exposed for panels that pace sibling
///   fetches off the same rhythm.
/// - refresh(): SILENT reload handed to tabs/drawers so every write converges
///   into this one snapshot without flickering the spinner.
export function useOverview({ onNotice } = {}) {
  const [overview, setOverview] = useState(null);
  const [loading, setLoading] = useState(false);
  const timer = useRef(null);
  const alive = useRef(true);
  const busy = overviewBusy(overview);

  // Keep `load` identity STABLE regardless of the onNotice prop identity: a
  // caller passing a fresh inline arrow (tests, memo boundaries) must not
  // re-arm the mount effect into a fetch→setState→render loop.
  const noticeRef = useRef(onNotice);
  useEffect(() => {
    noticeRef.current = onNotice;
  }, [onNotice]);

  const load = useCallback(async (silent) => {
    if (!silent) {
      setLoading(true);
    }
    try {
      const j = await apiGet('/api/project/overview');
      if (alive.current) {
        setOverview(j || { goals: [], backlog: [] });
      }
    } catch (e) {
      if (!silent && alive.current) {
        const notify = noticeRef.current;
        if (notify) {
          notify('获取项目总览失败: ' + (e && e.message));
        }
      }
    } finally {
      if (alive.current && !silent) {
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    load(false);
    return () => {
      alive.current = false;
      clearInterval(timer.current);
    };
  }, [load]);

  // Adaptive poll: fast while a todo runs, slow otherwise; re-armed on flip.
  useEffect(() => {
    timer.current = setInterval(() => load(true), busy ? POLL_BUSY_MS : POLL_IDLE_MS);
    return () => clearInterval(timer.current);
  }, [busy, load]);

  const refresh = useCallback(() => load(true), [load]);
  return { overview, loading, busy, refresh };
}
