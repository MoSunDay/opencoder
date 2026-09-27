// Shared project snapshot. TODO status is user-managed, not execution-driven.

import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../api.js';
const POLL_IDLE_MS = 8000;

/// useOverview({ onNotice }) → { overview, loading, refresh }
/// - overview: latest snapshot; null before the first load lands, and the
///   canonical empty catalog shape for empty payloads.
/// - loading: true only during the loud (non-silent) load — panels wrap
///   their body in <Spin spinning={loading}>.
/// - refresh(): SILENT reload handed to tabs/drawers so every write converges
///   into this one snapshot without flickering the spinner.
export function useOverview({ onNotice } = {}) {
  const [overview, setOverview] = useState(null);
  const [error, setError] = useState('');
  const [updated, setUpdated] = useState(null);
  const serial = useRef(0);
  const [loading, setLoading] = useState(false);
  const timer = useRef(null);
  const alive = useRef(true);

  // Keep `load` identity STABLE regardless of the onNotice prop identity: a
  // caller passing a fresh inline arrow (tests, memo boundaries) must not
  // re-arm the mount effect into a fetch→setState→render loop.
  const noticeRef = useRef(onNotice);
  useEffect(() => {
    noticeRef.current = onNotice;
  }, [onNotice]);

  const load = useCallback(async (silent) => {
    const request = ++serial.current;
    if (!silent) {
      setLoading(true);
    }
    try {
      const j = await apiGet('/api/project/overview');
      if (alive.current && request === serial.current) {
        setOverview(j || { goals: [], standalone_milestones: [], standalone_initiatives: [], backlog: [] });
        setError('');
        setUpdated(Date.now());
      }
    } catch (e) {
      if (alive.current && request === serial.current) setError(`获取项目总览失败: ${e.message}`);
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

  useEffect(() => {
    timer.current = setInterval(() => load(true), POLL_IDLE_MS);
    return () => clearInterval(timer.current);
  }, [load]);

  const refresh = useCallback(() => load(true), [load]);
  return { overview, loading, refresh, error, updated };
}
