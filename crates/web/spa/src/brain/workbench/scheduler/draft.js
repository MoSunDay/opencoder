import { useLayoutEffect, useRef, useState } from 'react';
import { convertPlan, inputRows, newVersion } from './model.js';
export const draftKey = (owner, version) => `oc:brain:scheduler-draft:v7:${encodeURIComponent(owner)}:${version ? `${version.id}@${version.version + 1}` : 'new'}`;
export function createDraft(version) {
  const next = newVersion(version);
  return { version: next, engineering: inputRows(next.plan.inputs) };
}
export function readDraft(key, version, storage = localStorage) {
  const raw = storage.getItem(key);
  if (raw === null) {
    const previous = storage.getItem(key.replace(':v7:', ':v5:'));
    if (previous !== null) {
      try {
        const legacy = JSON.parse(previous);
        if (legacy?.version?.plan?.schema_version === 6) return { ...legacy, version: { ...legacy.version, plan: convertPlan(legacy.version.plan) }, layout: {} };
      } catch { throw new Error('旧版浏览器草稿已损坏，原文已保留'); }
    }
    return createDraft(version);
  }
  let draft;
  try { draft = JSON.parse(raw); } catch { throw new Error('浏览器草稿已损坏，原文已保留'); }
  if (!draft?.version?.id || draft.version.plan?.schema_version !== 7 || !Array.isArray(draft.version.plan.nodes)
    || !Array.isArray(draft.engineering) || typeof draft.version.plan.title !== 'string' || typeof draft.version.plan.objective !== 'string') throw new Error('浏览器草稿格式无效，原文已保留');
  const plan = draft.version.plan;
  if (!Array.isArray(plan.layers) || !Array.isArray(plan.transitions) || plan.layers.length > 32 || plan.nodes.length > 256 ||
    plan.layers.some((layer) => ['layer_id', 'title', 'objective', 'success_criteria'].some((field) => typeof layer[field] !== 'string')) ||
    plan.nodes.some((node) => ['node_id', 'layer_id', 'title', 'objective', 'capability_id'].some((field) => typeof node[field] !== 'string')) ||
    plan.transitions.some((edge) => ['from', 'to', 'condition'].some((field) => typeof edge[field] !== 'string'))) throw new Error('浏览器画布草稿格式无效，原文已保留');
  return draft;
}
export function useDraft(key, version) {
  const [initial] = useState(() => { try { return { draft: readDraft(key, version), error: '' }; } catch (error) { return { draft: null, error: error.message }; } });
  const [draft, setDraft] = useState(initial.draft); const [error, setError] = useState(initial.error); const cleared = useRef(false);
  const persist = () => {
    if (!draft) return false;
    try { localStorage.setItem(key, JSON.stringify(draft)); setError(''); return true; }
    catch (error) { setError(`草稿未写入浏览器：${error.message}`); return false; }
  };
  useLayoutEffect(() => { if (!cleared.current && draft) persist(); }, [key, draft]);
  const clear = () => { localStorage.removeItem(key); cleared.current = true; };
  const discard = () => {
    try { const raw = localStorage.getItem(key); if (raw !== null) localStorage.setItem(`${key}:backup:${Date.now()}`, raw); localStorage.removeItem(key); setDraft(createDraft(version)); setError(''); }
    catch (error) { setError(error.message); }
  };
  return { draft, setDraft, error, persist, clear, discard };
}
