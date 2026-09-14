import { useLayoutEffect, useRef, useState } from 'react';
import { checkDraftPlan, createDraft } from './model.js';

export function draftKey(owner, version) {
  return `oc:brain:plan-draft:${encodeURIComponent(owner)}:${version ? `${version.id}@${version.version}` : 'new'}`;
}
export function readDraft(key, version, storage = localStorage) {
  const raw = storage.getItem(key);
  if (raw === null) return createDraft(version);
  const draft = JSON.parse(raw);
  if (!draft?.version?.id || !Array.isArray(draft.version.plan?.steps) || !draft.positions || !draft.raw || !draft.metadata) throw new Error('浏览器草稿格式无效，原始缓存已保留');
  checkDraftPlan(draft.version.plan);
  return draft;
}
export function useDraft(key, version) {
  const [initial] = useState(() => {
    try { return { draft: readDraft(key, version), error: '' }; }
    catch (e) { return { draft: null, error: e.message }; }
  });
  const [draft, setDraft] = useState(initial.draft);
  const [error, setError] = useState(initial.error);
  const cleared = useRef(false);
  const persist = () => {
    if (!draft) return false;
    try { cleared.current = false; localStorage.setItem(key, JSON.stringify(draft)); setError(''); return true; }
    catch (e) { setError(`草稿未写入浏览器：${e.message}`); return false; }
  };
  useLayoutEffect(() => { if (!cleared.current) persist(); }, [key, draft]); // synchronous flush before navigation/reload
  const retry = () => { try { cleared.current = false; setDraft(readDraft(key, version)); setError(''); } catch (e) { setError(e.message); } };
  const clear = () => { cleared.current = true; localStorage.removeItem(key); };
  return { draft, setDraft, error, persist, retry, clear };
}
