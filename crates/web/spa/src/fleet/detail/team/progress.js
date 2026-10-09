import { useEffect, useRef, useState } from 'react';
import { apiGet } from '../../../api.js';
import { decodeRecord } from './model.js';

// Runtime writes immutable plan/results before publishing a completed round.
// Inspect only the unfinished round, and at most eight summary records per poll.
export function useRoundProgress({ id, number, running }) {
  const key = JSON.stringify([id, number]);
  const cache = useRef(null);
  const [state, setState] = useState({ key, busy: true });
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    let timer;
    if (cache.current?.key !== key) cache.current = { key, step: 0, phase: 'planning' };
    const current = cache.current;
    const publish = (extra = {}) => { if (!controller.signal.aborted) setState({ ...current, ...extra }); };
    const read = async (field) => {
      const query = new URLSearchParams({ field, offset: '0' });
      try { return decodeRecord(await apiGet(`/api/executions/${encodeURIComponent(id)}/detail-field?${query}`, { signal: controller.signal })); }
      catch (error) { if (error.status === 404) return null; throw error; }
    };
    const load = async () => {
      try {
        if (!current.plan) {
          const plan = await read(`team.turn.${number}.plan`);
          if (controller.signal.aborted) return;
          if (plan?.large) { current.large = true; publish({ busy: false }); return; }
          if (!plan) { publish({ busy: false }); return; }
          if (!Array.isArray(plan.value.participants) || plan.value.participants.some((item) => typeof item !== 'string')) throw new Error('讨论安排的成员格式无效');
          current.plan = plan.value; current.phase = 'speaking'; publish({ busy: false });
        }
        for (let count = 0; count < 8 && current.phase !== 'closing'; count += 1) {
          const summary = await read(`team.turn.${number}.sub.${current.step}.summary`);
          if (controller.signal.aborted) return;
          if (!summary) break;
          if (summary.large) { current.summaryLarge = true; break; }
          if (typeof summary.value.aligned !== 'boolean') throw new Error('队长小结的状态格式无效');
          if (summary.value.aligned) current.phase = 'closing';
          else { current.step += 1; current.phase = 'clarifying'; }
        }
        publish({ busy: false });
      } catch (error) { publish({ busy: false, error: error.message || '读取当前讨论失败' }); }
      finally { if (running && !controller.signal.aborted && !current.large && !current.summaryLarge && current.phase !== 'closing') timer = setTimeout(load, 3000); }
    };
    publish({ busy: !current.plan }); load();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [id, number, running, key, revision]);
  return { ...(state.key === key ? state : { busy: true }), retry: () => setRevision((v) => v + 1) };
}
